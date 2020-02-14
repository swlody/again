#[macro_use]
extern crate static_assertions;

use std::{
    f32,
    ffi::OsStr,
    io::{Error, Result},
    mem::MaybeUninit,
    ptr,
};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use winapi::{
    shared::{minwindef::*, mmreg::*, windef::*, winerror::*},
    um::{
        cguid::*, debugapi::OutputDebugStringW, dsound::*, libloaderapi::GetModuleHandleW,
        wingdi::*, winuser::*, xinput::*,
    },
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
struct DisplayBuffer {
    info: BITMAPINFO,
    memory: Vec<Pixel>,
    current_offset: i32,
}

#[cfg(windows)]
impl DisplayBuffer {
    fn step_render(&mut self, step_by: i32) {
        let width = self.info.bmiHeader.biWidth;
        let height = self.info.bmiHeader.biHeight;
        assert!(width > 0 && height > 0);

        assert!(self.memory.len() == height as usize * width as usize);
        for (i, pixel) in self.memory.iter_mut().enumerate() {
            assert!(i < i32::max_value() as usize);
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

    /// Requires that `device_context` is a valid device context and that info is valid
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
            self.memory.as_ptr() as *const _,
            &self.info as *const _,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }
}

#[cfg(windows)]
const_assert!(std::mem::size_of::<BITMAPINFOHEADER>() < u32::max_value() as usize);

#[cfg(windows)]
static mut DISPLAY_BUFFER: DisplayBuffer = DisplayBuffer {
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

static mut HZ: u16 = 512;

#[cfg(windows)]
/// `unsafe` precondition: must be called from main thread
unsafe fn handle_key_press(vk_code: WPARAM, l_param: LPARAM) {
    assert!(vk_code < i32::max_value() as usize);
    let _was_down = (l_param & (1 << 30)) != 0;
    let is_down = (l_param & (1 << 31)) == 0;

    let alt_key_pressed = (l_param & (1 << 29)) != 0;
    match vk_code as i32 {
        VK_UP if is_down => HZ = HZ.saturating_add(64),
        VK_DOWN if is_down => HZ = HZ.saturating_sub(64),
        VK_ESCAPE => RUNNING = false,
        VK_F4 if alt_key_pressed => RUNNING = false,
        _ => (),
    }
}

#[cfg(windows)]
#[must_use]
fn initialize_direct_sound(
    window: HWND,
    buffer_size: u32,
    samples_per_second: u32,
) -> (LPDIRECTSOUND, LPDIRECTSOUNDBUFFER, LPDIRECTSOUNDBUFFER) {
    let mut direct_sound_ptr: LPDIRECTSOUND = ptr::null_mut();
    if unsafe {
        DirectSoundCreate(
            // Null for device default
            ptr::null(),
            // Out param for DirectSound object
            &mut direct_sound_ptr as *mut _,
            // Must be null
            ptr::null_mut(),
        )
    } != DS_OK
    {
        panic!("Failed to create DirectSound");
    }

    let direct_sound = unsafe {
        let direct_sound = direct_sound_ptr.as_ref().unwrap();
        if direct_sound.SetCooperativeLevel(
            // window handle
            window,
            // flags
            DSSCL_PRIORITY,
        ) != DS_OK
        {
            panic!("Failed to set DirectSound cooperative level")
        }
        direct_sound
    };

    let primary_buffer_description = DSBUFFERDESC {
        // Size of structure, in bytes
        dwSize: std::mem::size_of::<DSBUFFERDESC>() as DWORD,
        // Flags
        dwFlags: DSBCAPS_PRIMARYBUFFER,
        // Must be 0 for primary buffer
        dwBufferBytes: 0,
        // Must be 0
        dwReserved: 0,
        // Must be null for primary buffer
        lpwfxFormat: ptr::null_mut(),
        // Must be GUID_NULL since 3D flag is not set
        guid3DAlgorithm: GUID_NULL,
    };
    let mut primary_buffer_ptr: LPDIRECTSOUNDBUFFER = ptr::null_mut();
    if unsafe {
        direct_sound.CreateSoundBuffer(
            // DSBUFFERDESC object describing the buffer
            &primary_buffer_description as *const _,
            // Out pointer for allocated buffer
            &mut primary_buffer_ptr as *mut _,
            // Must be null
            ptr::null_mut(),
        )
    } != DS_OK
    {
        panic!("Failed to create primary DirectSound buffer");
    }
    assert!(!primary_buffer_ptr.is_null());

    let mut wav_format = {
        const BITS_PER_BYTE: u16 = 8;

        let num_channels = 2;
        let bits_per_sample = 16;
        // product of channels and bits per sample divided by bits per byte
        let block_align = num_channels * bits_per_sample / BITS_PER_BYTE;
        // product of sample rate and block align
        let avg_bytes_per_sec = samples_per_second * block_align as u32;

        WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM,
            nChannels: 2,
            nSamplesPerSec: samples_per_second,
            nAvgBytesPerSec: avg_bytes_per_sec,
            nBlockAlign: block_align,
            wBitsPerSample: bits_per_sample,
            // Ignored for PCM
            cbSize: 0,
        }
    };

    if unsafe { (*primary_buffer_ptr).SetFormat(&wav_format as *const _) } != DS_OK {
        panic!("Failed to set primary sound buffer format");
    }

    let secondary_buffer_description = DSBUFFERDESC {
        // Again, size of the structure
        dwSize: std::mem::size_of::<DSBUFFERDESC>() as DWORD,
        // Not the primary buffer
        dwFlags: 0,
        // For secondary buffer: size of buffer to allocate
        dwBufferBytes: buffer_size,
        // Must be 0
        dwReserved: 0,
        // For secondary buffer, pointer to format description
        lpwfxFormat: &mut wav_format as *mut _,
        // Must be GUID_NULL since 3D flag is not set
        guid3DAlgorithm: GUID_NULL,
    };
    let mut secondary_buffer_ptr: LPDIRECTSOUNDBUFFER = ptr::null_mut();
    if unsafe {
        direct_sound.CreateSoundBuffer(
            // DSBUFFERDESC object describing the buffer
            &secondary_buffer_description as *const _,
            // Out pointer for allocated buffer
            &mut secondary_buffer_ptr as *mut _,
            // Must be null
            ptr::null_mut(),
        )
    } != DS_OK
    {
        panic!("Failed to create secondary sound buffer");
    }
    assert!(!secondary_buffer_ptr.is_null());

    // Successfully allocated our buffers - return their pointers
    (direct_sound_ptr, primary_buffer_ptr, secondary_buffer_ptr)
}

#[cfg(windows)]
struct SoundOutput {
    volume: f32,
    wave_period: f32,
    t_sin: f32,
    running_sample_index: u32,
    buffer_size: u32,
    latency_sample_count: u32,
    sample_rate: u16,
    bytes_per_sample: u8,
}

#[cfg(windows)]
impl SoundOutput {
    fn render_to_buffer(
        &mut self,
        buffer: &IDirectSoundBuffer,
        byte_to_lock: u32,
        bytes_to_write: u32,
    ) {
        let mut region_1_ptr: LPVOID = ptr::null_mut();
        let mut region_1_size: DWORD = 0;
        let mut region_2_ptr: LPVOID = ptr::null_mut();
        let mut region_2_size: DWORD = 0;
        let value = unsafe {
            buffer.Lock(
                byte_to_lock,
                bytes_to_write,
                &mut region_1_ptr as *mut _,
                &mut region_1_size as *mut _,
                &mut region_2_ptr as *mut _,
                &mut region_2_size as *mut _,
                0,
            )
        };
        if value != DS_OK {
            println!("{}", Error::last_os_error());
            match value {
                -2_005_401_450 => println!("Buffer lost"),
                -2_005_401_550 => println!("Invalid call"),
                -2_147_024_809 => {
                    println!("Invalid parameter: {} {}", byte_to_lock, bytes_to_write)
                }
                -2_005_401_530 => println!("Priority level needed"),
                _ => (),
            }
            dbg!("Failed to lock DirectSound buffer");
            return;
        }

        let mut sample_out = region_1_ptr as *mut i16;
        let region_1_sample_count = region_1_size / u32::from(self.bytes_per_sample);
        for _ in 0..region_1_sample_count {
            let sample_value = (self.t_sin.sin() * self.volume) as i16;

            unsafe {
                *sample_out = sample_value;
                sample_out = sample_out.add(1);
                *sample_out = sample_value;
                sample_out = sample_out.add(1);
            }

            self.t_sin += 2.0 * f32::consts::PI * 1.0 / self.wave_period;
            self.running_sample_index = self.running_sample_index.wrapping_add(1);
        }

        sample_out = region_2_ptr as *mut i16;
        let region_2_sample_count = region_2_size / u32::from(self.bytes_per_sample);
        for _ in 0..region_2_sample_count {
            let sample_value = (self.t_sin.sin() * self.volume) as i16;

            unsafe {
                *sample_out = sample_value;
                sample_out = sample_out.add(1);
                *sample_out = sample_value;
                sample_out = sample_out.add(1);
            }

            self.t_sin += 2.0 * f32::consts::PI * 1.0 / self.wave_period;
            self.running_sample_index = self.running_sample_index.wrapping_add(1);
        }

        unsafe { buffer.Unlock(region_1_ptr, region_1_size, region_2_ptr, region_2_size) };
    }
}

#[cfg(windows)]
/// `unsafe` precondition: must be called from main thread
unsafe extern "system" fn main_window_callback(
    window: HWND,
    message: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    DISPLAY_BUFFER.resize_dib_section(1280, 720);

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
            DISPLAY_BUFFER.draw_to_window(device_context, dimension.width, dimension.height);
            EndPaint(window, &paint as *const _);
        }

        _ => result = DefWindowProcW(window, message, w_param, l_param),
    }

    result
}

#[cfg(windows)]
fn main() -> Result<()> {
    let window_name = win32_string("HandmadeWindowClass");
    let title = win32_string("Handmade!");

    let hinstance = unsafe {
        GetModuleHandleW(
            // null to return handle to calling process
            ptr::null(),
        )
    };
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

    let mut sound_output = {
        let volume = 4000.0;
        let sample_rate = 48000;
        let bytes_per_sample = std::mem::size_of::<WORD>() as u8 * 2;
        let buffer_size = u32::from(sample_rate) * u32::from(bytes_per_sample);

        SoundOutput {
            sample_rate,
            volume,
            wave_period: f32::from(sample_rate) / f32::from(unsafe { HZ }),
            buffer_size,
            latency_sample_count: u32::from(sample_rate) / 60,
            bytes_per_sample,
            running_sample_index: 0,
            t_sin: 0.0,
        }
    };

    // We'll only be writing to the secondary buffer, but need to retain the other two pointers to release them
    let (direct_sound_ptr, primary_buffer_ptr, secondary_buffer_ptr) = initialize_direct_sound(
        window,
        sound_output.buffer_size,
        u32::from(sound_output.sample_rate),
    );

    unsafe {
        let device_context = GetDC(window);

        let secondary_buffer = secondary_buffer_ptr.as_ref().unwrap();

        let bytes_to_write =
            sound_output.latency_sample_count * u32::from(sound_output.bytes_per_sample);
        sound_output.render_to_buffer(secondary_buffer, 0, bytes_to_write);
        secondary_buffer.Play(0, 0, DSBPLAY_LOOPING);

        RUNNING = true;
        while RUNNING {
            let mut message = MaybeUninit::<MSG>::uninit();
            while PeekMessageW(message.as_mut_ptr(), ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                let message = message.assume_init();
                if message.message == WM_QUIT {
                    RUNNING = false;
                }

                TranslateMessage(&message as *const _);
                DispatchMessageW(&message as *const _);
            }

            // Handle gamepad input
            for controller_index in 0..XUSER_MAX_COUNT {
                let mut controller_state = MaybeUninit::<XINPUT_STATE>::uninit();
                if XInputGetState(controller_index, controller_state.as_mut_ptr()) == ERROR_SUCCESS
                {
                    let controller_state = controller_state.assume_init();
                    let pad = &controller_state.Gamepad;
                    let _up_pressed = (pad.wButtons & XINPUT_GAMEPAD_DPAD_UP) != 0;
                    let _stick_x = pad.sThumbLX;
                } else {
                    // Controller not available
                }
            }

            // Render image
            DISPLAY_BUFFER.step_render(1);

            let mut play_cursor: DWORD = 0;
            let mut write_cursor: DWORD = 0;
            if secondary_buffer
                .GetCurrentPosition(&mut play_cursor as *mut _, &mut write_cursor as *mut _)
                != DS_OK
            {
                panic!("Failed to get current DirectSound buffer position");
            }

            let byte_to_lock = (sound_output.running_sample_index
                * u32::from(sound_output.bytes_per_sample))
                % sound_output.buffer_size;
            let target_cursor = (play_cursor
                + (sound_output.latency_sample_count * u32::from(sound_output.bytes_per_sample)))
                % sound_output.buffer_size;
            let bytes_to_write = if byte_to_lock > target_cursor {
                sound_output.buffer_size - byte_to_lock + target_cursor
            } else {
                target_cursor - byte_to_lock
            };

            sound_output.wave_period = f32::from(sound_output.sample_rate) / f32::from(HZ);
            sound_output.render_to_buffer(secondary_buffer, byte_to_lock, bytes_to_write);

            // Draw image to window
            let dimension = get_window_dimension(window);
            DISPLAY_BUFFER.draw_to_window(device_context, dimension.width, dimension.height);
        }

        ReleaseDC(window, device_context);

        (*direct_sound_ptr).Release();
        (*primary_buffer_ptr).Release();
        secondary_buffer.Release();

        if DestroyWindow(window) == 0 {
            dbg!("Failed to destroy window");
        }
    }

    Ok(())
}
