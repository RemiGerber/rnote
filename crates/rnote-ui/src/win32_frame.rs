//! Native window management for the client side decorated app windows on Windows.
//!
//! GTK 4.18 dropped its Aero Snap emulation (GTK MR !7902) without a native replacement yet (GTK MR !4252).
//! Instead of emulating it again, we answer `WM_NCHITTEST` ourselves so that Windows treats the empty
//! header bar area as the title bar and the window edges as resize borders. Windows then runs its own
//! move/resize loop, which gives us snapping to edges and corners, Snap Assist, double-click to maximize
//! and drag-down to restore. For that (and Win+Arrow) Windows also needs to see the regular
//! frame window styles, so those are added and the frame they would bring along is removed again
//! in `WM_NCCALCSIZE`.

// Imports
use gdk4_win32::{MSG, Win32Display, Win32MessageFilterReturn, Win32Surface};
use gtk4::prelude::*;
use windows::Win32::Foundation::{HWND, LPARAM, POINT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_STYLE, GetWindowLongPtrW, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT,
    HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, NCCALCSIZE_PARAMS, SM_CXFRAME,
    SM_CXPADDEDBORDER, SM_CYFRAME, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WM_NCCALCSIZE, WM_NCHITTEST,
    WM_STYLECHANGING, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

/// The styles Windows checks before it offers snapping, Win+Arrow and the minimize/maximize animations.
const NATIVE_STYLES: u32 =
    WS_THICKFRAME.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
/// Thickness of the native resize border along the inside edges of the window, in logical pixels.
const RESIZE_BORDER: f64 = 6.0;
/// Key under which the message filter handle is stored on the window. Dropping the handle removes the filter.
const FILTER_KEY: &str = "rn-win32-frame-filter";

/// Hand moving, resizing and snapping of the window over to Windows.
///
/// Must be called after the window is realized, because it needs the native window handle.
pub(crate) fn install(window: &gtk4::Window) {
    let Some(surface) = window.surface().and_downcast::<Win32Surface>() else {
        tracing::warn!("Installing native frame handling failed, surface is not a Win32Surface");
        return;
    };
    let Ok(display) = window.display().downcast::<Win32Display>() else {
        tracing::warn!("Installing native frame handling failed, display is not a Win32Display");
        return;
    };
    let hwnd = surface.handle();
    let window_weak = window.downgrade();

    let filter = display.add_filter(move |_, msg, ret| {
        if msg.hwnd != hwnd {
            return Win32MessageFilterReturn::Continue;
        }
        let Some(window) = window_weak.upgrade() else {
            return Win32MessageFilterReturn::Continue;
        };
        match handle_message(&window, msg) {
            Some(result) => {
                *ret = result as i32;
                Win32MessageFilterReturn::Remove
            }
            None => Win32MessageFilterReturn::Continue,
        }
    });
    // SAFETY: the data is only ever accessed as `Win32DisplayFilterHandle` under this key.
    unsafe {
        window.set_data(FILTER_KEY, filter);
    }

    // Add the styles and let Windows recalculate the frame, which `WM_NCCALCSIZE` then removes again.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style | NATIVE_STYLES as isize);
        if let Err(e) = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        ) {
            tracing::warn!("Applying native frame window styles failed, Err: {e:?}");
        }
    }
}

/// Returns `Some(result)` when the message is fully handled here and should not reach GTK.
fn handle_message(window: &gtk4::Window, msg: &mut MSG) -> Option<isize> {
    let hwnd = msg.hwnd;
    match msg.message {
        WM_NCHITTEST => hit_test(window, hwnd, msg.lParam).map(|ht| ht as isize),
        // Make the entire window the client area, otherwise Windows draws its own frame and title bar
        // because of the added styles.
        WM_NCCALCSIZE if msg.wParam.0 != 0 => {
            if unsafe { IsZoomed(hwnd) }.as_bool() {
                // Maximized windows with a thick frame are placed so that the frame lies outside of the monitor.
                // Remove it from the client area, else the content gets cut off at the monitor edges.
                let params = unsafe { &mut *(msg.lParam.0 as *mut NCCALCSIZE_PARAMS) };
                let dpi = unsafe { GetDpiForWindow(hwnd) };
                let padded = unsafe { GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) };
                let frame_x = unsafe { GetSystemMetricsForDpi(SM_CXFRAME, dpi) } + padded;
                let frame_y = unsafe { GetSystemMetricsForDpi(SM_CYFRAME, dpi) } + padded;
                let rect = &mut params.rgrc[0];
                rect.left += frame_x;
                rect.right -= frame_x;
                rect.top += frame_y;
                rect.bottom -= frame_y;
            }
            Some(0)
        }
        // GTK recalculates the window styles on some state changes, make sure ours survive.
        // The message is only modified, not consumed.
        WM_STYLECHANGING if msg.wParam.0 as i32 == GWL_STYLE.0 => {
            let style = unsafe { &mut *(msg.lParam.0 as *mut STYLESTRUCT) };
            style.styleNew |= NATIVE_STYLES;
            None
        }
        _ => None,
    }
}

/// Tell Windows what part of the window is under the pointer.
///
/// `None` leaves the hit test to GTK, which treats it as client area (or click-through on the CSD shadow).
fn hit_test(window: &gtk4::Window, hwnd: HWND, lparam: LPARAM) -> Option<u32> {
    if window.is_fullscreen() {
        return None;
    }
    // Screen coordinates in physical pixels, signed because of monitors left of / above the primary one.
    let mut point = POINT {
        x: (lparam.0 & 0xffff) as i16 as i32,
        y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    };
    if !unsafe { ScreenToClient(hwnd, &mut point) }.as_bool() {
        return None;
    }
    // Physical client pixels -> logical surface coordinates -> window widget coordinates.
    // The surface is larger than the window widget by the CSD shadow margin.
    let scale = window.surface()?.scale();
    let (transform_x, transform_y) = window.surface_transform();
    let x = point.x as f64 / scale - transform_x;
    let y = point.y as f64 / scale - transform_y;
    let (width, height) = (window.width() as f64, window.height() as f64);
    if x < 0.0 || y < 0.0 || x >= width || y >= height {
        // On the CSD shadow, GTK has its own resize handles there.
        return None;
    }

    if window.is_resizable() && !window.is_maximized() {
        let left = x < RESIZE_BORDER;
        let right = x >= width - RESIZE_BORDER;
        let top = y < RESIZE_BORDER;
        let bottom = y >= height - RESIZE_BORDER;
        let edge = match (top, bottom, left, right) {
            (true, _, true, _) => Some(HTTOPLEFT),
            (true, _, _, true) => Some(HTTOPRIGHT),
            (_, true, true, _) => Some(HTBOTTOMLEFT),
            (_, true, _, true) => Some(HTBOTTOMRIGHT),
            (true, ..) => Some(HTTOP),
            (_, true, ..) => Some(HTBOTTOM),
            (.., true, _) => Some(HTLEFT),
            (.., true) => Some(HTRIGHT),
            _ => None,
        };
        if edge.is_some() {
            return edge;
        }
    }

    is_caption(window, x, y).then_some(HTCAPTION)
}

/// Whether the point is on a part of a header bar that should drag the window,
/// mimicking what `GtkWindowHandle` would do with a pointer press there.
fn is_caption(window: &gtk4::Window, x: f64, y: f64) -> bool {
    let Some(mut widget) = window.pick(x, y, gtk4::PickFlags::DEFAULT) else {
        return false;
    };
    loop {
        if is_interactive(&widget) {
            return false;
        }
        if widget.is::<gtk4::WindowHandle>() || widget.is::<adw::HeaderBar>() {
            return true;
        }
        match widget.parent() {
            Some(parent) => widget = parent,
            None => return false,
        }
    }
}

/// Widgets that handle pointer presses themselves and must keep receiving them.
///
/// Extend this when some header bar widget stops reacting to clicks.
fn is_interactive(widget: &gtk4::Widget) -> bool {
    widget.is_focusable()
        || widget.is::<gtk4::Button>()
        || widget.is::<gtk4::MenuButton>()
        || widget.is::<gtk4::Editable>()
        // AdwTab, private type. Dragging tabs reorders/detaches them.
        || widget.css_name() == "tab"
}
