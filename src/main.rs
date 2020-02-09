use std::{
    ffi::OsStr,
    io::{Error, Result},
    mem::MaybeUninit,
    os::windows::ffi::OsStrExt,
    ptr,
};
use winapi::{
    shared::{minwindef::*, windef::*},
    um::{debugapi::OutputDebugStringW, libloaderapi::GetModuleHandleW, wingdi::*, winuser::*},
};

#[cfg(windows)]
fn win32_string(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[derive(Debug, Default, Clone, Copy)]
struct Pixel {
    b: u8,
    g: u8,
    r: u8,
    a: u8,
}

#[cfg(windows)]
struct Buffer {
    info: BITMAPINFO,
    memory: Vec<Pixel>,
    current_offset: i32,
}

#[cfg(windows)]
impl Buffer {
    fn step_render(&mut self, step_by: i32) {
        let width = self.info.bmiHeader.biWidth;
        let height = self.info.bmiHeader.biHeight;
        assert!(width > 0 && height > 0);

        assert!(self.memory.len() == height as usize * width as usize);
        for (i, pixel) in self.memory.iter_mut().enumerate() {
            assert!(i < u32::max_value() as usize);
            let x = i as i32 % width;
            let y = i as i32 / height;
            pixel.g = ((x ^ y) - self.current_offset) as u8;
        }

        self.current_offset += step_by;
    }

    fn resize_dib_section(&mut self, window_width: i32, window_height: i32) {
        assert!(window_width > 0 && window_height > 0);

        self.info.bmiHeader.biWidth = window_width;
        self.info.bmiHeader.biHeight = window_height;

        let new_size = window_width as usize * window_height as usize;
        if new_size != self.memory.len() {
            self.memory.resize_with(new_size, Default::default);
        }

        self.step_render(1);
    }

    /// Requires that device_context is a valid device context and that info is valid
    unsafe fn draw_to_window(&self, device_context: HDC, window_width: i32, window_height: i32) {
        StretchDIBits(
            device_context,
            0,
            0,
            window_width,
            window_height,
            0,
            0,
            self.info.bmiHeader.biWidth,
            self.info.bmiHeader.biHeight,
            self.memory.as_ptr() as *const winapi::ctypes::c_void,
            &self.info as *const BITMAPINFO,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

#[cfg(windows)]
static mut BUFFER: Buffer = Buffer {
    info: BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 0,
            biHeight: 0,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbReserved: 0,
        }],
    },
    memory: Vec::new(),
    current_offset: 0,
};

static mut RUNNING: bool = false;

#[cfg(windows)]
struct WindowDimension {
    width: i32,
    height: i32,
}

#[cfg(windows)]
fn get_window_dimension(window: HWND) -> WindowDimension {
    let mut client_rect = MaybeUninit::<RECT>::uninit();
    unsafe { GetClientRect(window, client_rect.as_mut_ptr()) };

    let client_rect = unsafe { client_rect.assume_init() };
    WindowDimension {
        width: client_rect.right - client_rect.left,
        height: client_rect.bottom - client_rect.top,
    }
}

#[cfg(windows)]
fn handle_key_press(_vk_code: WPARAM, _l_param: LPARAM) {
    todo!();
}

#[cfg(windows)]
/// Must be called from main thread
unsafe extern "system" fn main_window_callback(
    window: HWND,
    message: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    BUFFER.resize_dib_section(1280, 720);

    let mut result = 0;
    match message {
        WM_SIZE => OutputDebugStringW(win32_string("WM_SIZE").as_ptr()),
        WM_CLOSE | WM_DESTROY => RUNNING = false,
        WM_ACTIVATEAPP => OutputDebugStringW(win32_string("WM_ACTIVATEAPP").as_ptr()),
        WM_KEYUP | WM_KEYDOWN | WM_SYSKEYUP | WM_SYSKEYDOWN => handle_key_press(w_param, l_param),
        WM_PAINT => {
            let mut paint = MaybeUninit::<PAINTSTRUCT>::uninit();
            let device_context = BeginPaint(window, paint.as_mut_ptr());
            let paint = paint.assume_init();
            let dimension = get_window_dimension(window);
            BUFFER.draw_to_window(device_context, dimension.width, dimension.height);
            EndPaint(window, &paint as *const PAINTSTRUCT);
        }

        _ => result = DefWindowProcW(window, message, w_param, l_param),
    }

    result
}

#[cfg(windows)]
fn main() -> Result<()> {
    let window_name = win32_string("HandmadeWindowClass");
    let title = win32_string("Handmade!");

    let hinstance = unsafe { GetModuleHandleW(ptr::null()) };
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(main_window_callback),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: ptr::null_mut(),
        hCursor: ptr::null_mut(),
        hbrBackground: ptr::null_mut(),
        lpszMenuName: ptr::null(),
        lpszClassName: window_name.as_ptr(),
    };

    assert!(unsafe { RegisterClassW(&window_class) } != 0);
    let window = unsafe {
        CreateWindowExW(
            0,
            window_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null_mut(),
        )
    };

    if window.is_null() {
        return Err(Error::last_os_error());
    }

    unsafe {
        RUNNING = true;
        while RUNNING {
            let mut message = MaybeUninit::<MSG>::uninit();
            while PeekMessageW(message.as_mut_ptr(), ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                let message = message.assume_init();
                if message.message == WM_QUIT {
                    RUNNING = false;
                }

                TranslateMessage(&message as *const MSG);
                DispatchMessageW(&message as *const MSG);
            }

            if !RUNNING {
                break;
            }

            let device_context = GetDC(window);
            let dimension = get_window_dimension(window);
            {
                BUFFER.step_render(1);
                BUFFER.draw_to_window(device_context, dimension.width, dimension.height);
            }
            ReleaseDC(window, device_context);
        }
    }

    Ok(())
}