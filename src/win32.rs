use std::{ffi::OsStr, io, mem::MaybeUninit, ptr};

use std::os::windows::ffi::OsStrExt;
use winapi::{
    shared::{minwindef::*, mmreg::*, windef::*, winerror::*},
    um::{
        cguid::*, debugapi::OutputDebugStringW, dsound::*, libloaderapi::GetModuleHandleW,
        profileapi::*, wingdi::*, winnt::*, winuser::*, xinput::*,
    },
};

use crate::game::*;

#[cfg(windows)]
fn win32_string(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

impl DisplayBuffer {
    fn resize_dib_section(&mut self, info: &mut BITMAPINFO, window_width: i32, window_height: i32) {
        assert!(window_width > 0 && window_height > 0);

        self.width = window_width;
        info.bmiHeader.biWidth = window_width;

        self.height = window_height;
        info.bmiHeader.biHeight = window_height;

        let new_size = window_width as usize * window_height as usize;
        if new_size != self.memory.len() {
            self.memory.resize_with(new_size, Default::default);
        }

        self.step_render(1);
    }

    /// Requires that `device_context` is a valid device context and that info is valid
    fn draw_to_window(
        &self,
        info: &BITMAPINFO,
        device_context: HDC,
        window_width: i32,
        window_height: i32,
    ) {
        let success = unsafe {
            StretchDIBits(
                // Destination device context handle
                device_context,
                // Upper left corner of destination rectangle coords
                0,
                0,
                // Dimensions of destination rectangle
                window_width,
                window_height,
                // Source rectangle of image
                0,
                0,
                // Dimensions of source image
                self.width,
                self.height,
                // Memory buffer of image
                self.memory.as_ptr() as *const _,
                // Pointer to BITMAPINFO containing DIB information
                info as *const _,
                // Image contains RGB values
                DIB_RGB_COLORS,
                // Copy source rectangle directly onto destination rectangle
                SRCCOPY,
            )
        };
        if success == 0 {
            panic!("Failed to draw image to window");
        }
    }
}

static mut RUNNING: bool = false;

struct WindowDimension {
    width: i32,
    height: i32,
}

fn get_window_dimension(window: HWND) -> WindowDimension {
    let client_rect = unsafe {
        let mut client_rect = MaybeUninit::uninit();
        let success = GetClientRect(
            // Handle to relevant window
            window,
            // Out pointer for client rect
            client_rect.as_mut_ptr(),
        );
        if success == 0 {
            panic!("Failed to get client rect");
        }
        client_rect.assume_init()
    };
    WindowDimension {
        width: client_rect.right - client_rect.left,
        height: client_rect.bottom - client_rect.top,
    }
}

static mut HZ: u16 = 512;

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

#[must_use]
/// Guaranteed to return valid (non-null) pointers
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
    assert!(!direct_sound_ptr.is_null());

    if unsafe {
        (*direct_sound_ptr).SetCooperativeLevel(
            // window handle
            window,
            // flags
            DSSCL_PRIORITY,
        )
    } != DS_OK
    {
        panic!("Failed to set DirectSound cooperative level")
    }

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
        (*direct_sound_ptr).CreateSoundBuffer(
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
        let avg_bytes_per_sec = samples_per_second * u32::from(block_align);

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
        (*direct_sound_ptr).CreateSoundBuffer(
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

struct SoundOutput {
    buffer_size: u32,
    running_sample_index: u32,
    latency_sample_count: u32,
    sample_rate: u16,
    bytes_per_sample: u16,
}

impl SoundOutput {
    fn clear_buffer(&self, destination_buffer: &mut IDirectSoundBuffer) {
        let mut region_1_ptr: LPVOID = ptr::null_mut();
        let mut region_1_size: DWORD = 0;
        let mut region_2_ptr: LPVOID = ptr::null_mut();
        let mut region_2_size: DWORD = 0;
        let success = unsafe {
            destination_buffer.Lock(
                0,
                self.buffer_size,
                &mut region_1_ptr as *mut _,
                &mut region_1_size as *mut _,
                &mut region_2_ptr as *mut _,
                &mut region_2_size as *mut _,
                0,
            )
        };
        if success != DS_OK {
            return;
        }

        let mut destination_sample = region_1_ptr as *mut u8;
        for _ in 0..region_1_size {
            unsafe {
                destination_sample.write(0);
                destination_sample = destination_sample.add(1);
            }
        }

        destination_sample = region_2_ptr as *mut u8;
        for _ in 0..region_2_size {
            unsafe {
                destination_sample.write(0);
                destination_sample = destination_sample.add(1);
            }
        }

        unsafe {
            destination_buffer.Unlock(region_1_ptr, region_1_size, region_2_ptr, region_2_size);
        }
    }

    fn fill_buffer(
        &mut self,
        destination_buffer: &mut IDirectSoundBuffer,
        source_buffer: &SoundBuffer,
        byte_to_lock: u32,
        bytes_to_write: u32,
    ) {
        let mut region_1_ptr: LPVOID = ptr::null_mut();
        let mut region_1_size: DWORD = 0;
        let mut region_2_ptr: LPVOID = ptr::null_mut();
        let mut region_2_size: DWORD = 0;
        let success = unsafe {
            destination_buffer.Lock(
                byte_to_lock,
                bytes_to_write,
                &mut region_1_ptr as *mut _,
                &mut region_1_size as *mut _,
                &mut region_2_ptr as *mut _,
                &mut region_2_size as *mut _,
                0,
            )
        };
        if success != DS_OK {
            // Failed to lock DirectSound buffer - this will happen if this function is called too often (currently only when building in release mode)
            return;
        }

        let region_1_sample_count = region_1_size as usize / self.bytes_per_sample as usize;
        let mut destination_sample = region_1_ptr as *mut i16;
        for i in (0..region_1_sample_count * 2).step_by(2) {
            unsafe {
                destination_sample.write(source_buffer.samples[i]);
                destination_sample = destination_sample.add(1);

                destination_sample.write(source_buffer.samples[i + 1]);
                destination_sample = destination_sample.add(1);
            }

            self.running_sample_index += 1;
        }

        let region_2_sample_count = region_2_size as usize / self.bytes_per_sample as usize;
        destination_sample = region_2_ptr as *mut i16;
        for i in (0..region_2_sample_count * 2).step_by(2) {
            unsafe {
                destination_sample.write(source_buffer.samples[i]);
                destination_sample = destination_sample.add(1);

                destination_sample.write(source_buffer.samples[i + 1]);
                destination_sample = destination_sample.add(1);
            }

            self.running_sample_index += 1;
        }

        unsafe {
            destination_buffer.Unlock(region_1_ptr, region_1_size, region_2_ptr, region_2_size);
        }
    }
}

const_assert!(std::mem::size_of::<BITMAPINFOHEADER>() < u32::max_value() as usize);

static mut DISPLAY_BUFFER: DisplayBuffer = DisplayBuffer {
    memory: Vec::new(),
    current_offset: 0,
    width: 1280,
    height: 720,
};

static mut BITMAP_INFO: BITMAPINFO = BITMAPINFO {
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
};

/// `unsafe` precondition: must be called from main thread
unsafe extern "system" fn main_window_callback(
    window: HWND,
    message: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    DISPLAY_BUFFER.resize_dib_section(&mut BITMAP_INFO, 1280, 720);

    let mut result = 0;
    match message {
        WM_SIZE => OutputDebugStringW(win32_string("WM_SIZE").as_ptr()),
        WM_CLOSE | WM_DESTROY => RUNNING = false,
        WM_ACTIVATEAPP => OutputDebugStringW(win32_string("WM_ACTIVATEAPP").as_ptr()),
        WM_KEYUP | WM_KEYDOWN | WM_SYSKEYUP | WM_SYSKEYDOWN => handle_key_press(w_param, l_param),
        WM_PAINT => {
            let mut paint = MaybeUninit::uninit();
            let device_context = BeginPaint(
                // Window handle
                window,
                // Out pointer for paint struct
                paint.as_mut_ptr(),
            );
            if device_context.is_null() {
                panic!("Could not begin paint");
            }
            let paint = paint.assume_init();
            let dimension = get_window_dimension(window);
            DISPLAY_BUFFER.draw_to_window(
                // Static can only be accessed from main thread
                &BITMAP_INFO,
                device_context,
                dimension.width,
                dimension.height,
            );
            EndPaint(
                // Winow handle
                window,
                // Paint struct returned from BeginPaint call
                &paint as *const _,
            );
        }

        _ => result = DefWindowProcW(window, message, w_param, l_param),
    }

    result
}

fn get_performance_counter() -> io::Result<LARGE_INTEGER> {
    unsafe {
        let mut begin_counter = MaybeUninit::uninit();
        if QueryPerformanceCounter(
            // Out pointer for performance counter
            begin_counter.as_mut_ptr(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(begin_counter.assume_init())
    }
}

#[cfg(target_arch = "x86")]
fn get_cycles() -> u64 {
    unsafe { core::arch::x86::_rdtsc() }
}

#[cfg(target_arch = "x86_64")]
fn get_cycles() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

pub fn win32_main() -> io::Result<()> {
    #[allow(unused_variables)]
    let perf_counter_frequency = unsafe {
        let mut perf_counter_frequency = MaybeUninit::uninit();
        if QueryPerformanceFrequency(perf_counter_frequency.as_mut_ptr()) == 0 {
            return Err(io::Error::last_os_error());
        }
        *perf_counter_frequency.assume_init().QuadPart()
    };

    let window_name = win32_string("HandmadeWindowClass");
    let title = win32_string("Handmade!");

    let hinstance = unsafe {
        GetModuleHandleW(
            // null to return handle to calling process
            ptr::null(),
        )
    };
    let window_class = WNDCLASSW {
        // Redraw if size changes
        style: CS_HREDRAW | CS_VREDRAW,
        // Callback for the window procedure
        lpfnWndProc: Some(main_window_callback),
        // Extra bytes to allocate after class structure
        cbClsExtra: 0,
        // Extra bytes to allocate after window instance
        cbWndExtra: 0,
        // Instance that contains the window procedure (this one)
        hInstance: hinstance,
        // Handle to class icon - null for system default
        hIcon: ptr::null_mut(),
        // Handle for class cursor - null for system default
        hCursor: ptr::null_mut(),
        // Handle to class background brush - null for application to paint its own background
        hbrBackground: ptr::null_mut(),
        // Class menu - null for none
        lpszMenuName: ptr::null(),
        // Class name - must match following call to CreateWindowEx
        lpszClassName: window_name.as_ptr(),
    };

    let (window, device_context) = unsafe {
        if RegisterClassW(
            // Pointer to WNDCLASS settings
            &window_class,
        ) == 0
        {
            panic!("Failed to register window class");
        }
        let window = CreateWindowExW(
            // Default window style
            WS_EX_LEFT,
            // Must be same as lpszClassName of previous call to RegisterClassW
            window_name.as_ptr(),
            // Title bar string
            title.as_ptr(),
            // Visible, tiled window
            WS_TILEDWINDOW | WS_VISIBLE,
            // Default horizontal position
            CW_USEDEFAULT,
            // Default vertical position
            CW_USEDEFAULT,
            // Default width
            CW_USEDEFAULT,
            // Default height
            CW_USEDEFAULT,
            // Parent window: null since no parent
            ptr::null_mut(),
            // Child window identifier - null
            ptr::null_mut(),
            // Instance of the module associated with the window
            hinstance,
            // Initial message to be sent to the window - null for no additional data
            ptr::null_mut(),
        );
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }

        // Get device constant assuming requires a valid window handle
        let device_context = GetDC(window);
        (window, device_context)
    };

    let mut sound_output = {
        let sample_rate = 48000;
        let bytes_per_sample = std::mem::size_of::<WORD>() as u16 * 2;
        let buffer_size = u32::from(sample_rate) * u32::from(bytes_per_sample);

        SoundOutput {
            sample_rate,
            buffer_size,
            latency_sample_count: u32::from(sample_rate) / 15,
            bytes_per_sample,
            running_sample_index: 0,
        }
    };

    // We'll only be writing to the secondary buffer, but need to retain the other two pointers to release them
    let (direct_sound_ptr, primary_buffer_ptr, secondary_buffer_ptr) = initialize_direct_sound(
        window,
        sound_output.buffer_size,
        u32::from(sound_output.sample_rate),
    );

    let secondary_buffer = unsafe { secondary_buffer_ptr.as_mut().unwrap() };
    sound_output.clear_buffer(secondary_buffer);
    unsafe {
        // Begin playing secondary buffer
        secondary_buffer.Play(
            // Must be 0
            0,
            // Must be 0
            0,
            // Circular buffer: looping
            DSBPLAY_LOOPING,
        );
    }

    // Static can only be accessed from main thread
    unsafe {
        RUNNING = true;
    }

    let mut sound_buffer = SoundBuffer {
        samples: vec![0; sound_output.buffer_size as usize],
        sample_count: 0,
        t_sin: 0.0,
        volume: 4000.0,
        sample_rate: sound_output.sample_rate,
    };

    #[allow(unused_mut, unused_variables)]
    let mut last_counter = get_performance_counter()?;
    #[allow(unused_mut, unused_variables)]
    let mut last_cycle_count = get_cycles();

    while unsafe { RUNNING } {
        unsafe {
            let mut message = MaybeUninit::uninit();
            while PeekMessageW(
                // Out pointer for message
                message.as_mut_ptr(),
                // Null to receive all messages meant for current thread
                ptr::null_mut(),
                // Next two params 0 to receive all available messages
                0,
                0,
                // Remove messages from queue after peek
                PM_REMOVE,
            ) != 0
            {
                // Non-zero return value means messages are available, so message is initialized
                let message = message.assume_init();
                if message.message == WM_QUIT {
                    RUNNING = false;
                }

                TranslateMessage(&message as *const _);
                DispatchMessageW(&message as *const _);
            }
        }

        // Handle gamepad input
        unsafe {
            for controller_index in 0..XUSER_MAX_COUNT {
                let mut controller_state = MaybeUninit::uninit();
                if XInputGetState(
                    // Index of controller
                    controller_index,
                    // Out pointer for state to set
                    controller_state.as_mut_ptr(),
                ) == ERROR_SUCCESS
                {
                    // Function succeeded - state is initialized
                    let controller_state = controller_state.assume_init();
                    let pad = &controller_state.Gamepad;
                    let _up_pressed = (pad.wButtons & XINPUT_GAMEPAD_DPAD_UP) != 0;
                    let _stick_x = pad.sThumbLX;
                } else {
                    // Controller not available
                }
            }
        }

        let mut play_cursor: DWORD = 0;
        let mut write_cursor: DWORD = 0;
        if {
            unsafe {
                secondary_buffer.GetCurrentPosition(
                    // Out pointer for play cursor
                    &mut play_cursor as *mut _,
                    // Out pointer for write cursor
                    &mut write_cursor as *mut _,
                )
            }
        } != DS_OK
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

        sound_buffer.sample_count =
            bytes_to_write as usize / sound_output.bytes_per_sample as usize;
        update_and_render(
            // static should only be accessed from main thread
            unsafe { &mut DISPLAY_BUFFER },
            &mut sound_buffer,
            unsafe { HZ },
        );

        sound_output.fill_buffer(
            secondary_buffer,
            &sound_buffer,
            byte_to_lock,
            bytes_to_write,
        );

        // Draw image to window
        let dimension = get_window_dimension(window);
        unsafe {
            // Static can only be accessed from main thread
            DISPLAY_BUFFER.draw_to_window(
                &BITMAP_INFO,
                device_context,
                dimension.width,
                dimension.height,
            );
        }

        let end_counter = get_performance_counter()?;
        // let counter_elapsed = unsafe { end_counter.QuadPart() - last_counter.QuadPart() };
        // let time_elapsed_in_ms = (counter_elapsed * 1000) / perf_counter_frequency;
        // let fps = perf_counter_frequency / counter_elapsed;

        let end_cycle_count = get_cycles();
        // let cycles_elapsed = end_cycle_count - last_cycle_count;
        // let million_cycles_per_frame = cycles_elapsed / 1_000_000;
        // println!(
        //     "Draw frame in {} ms, {} fps, {} million cycles per frame",
        //     time_elapsed_in_ms, fps, million_cycles_per_frame
        // );

        #[allow(unused_assignments)]
        {
            last_counter = end_counter;
            last_cycle_count = end_cycle_count;
        }
    }

    // Cleanup
    unsafe {
        ReleaseDC(
            // Window handle
            window,
            // Device context for given window handle
            device_context,
        );

        // Release buffers to free allocated memory
        (*direct_sound_ptr).Release();
        (*primary_buffer_ptr).Release();
        secondary_buffer.Release();

        // Destroy given window handle
        if DestroyWindow(window) == 0 {
            eprintln!("Failed to destroy window");
        }
    }

    Ok(())
}
