use std::f32;

#[derive(Debug, Default, Clone, Copy)]
pub struct Pixel {
    b: u8,
    g: u8,
    r: u8,
    a: u8,
}

pub struct DisplayBuffer {
    pub memory: Vec<Pixel>,
    pub current_offset: i32,
    pub width: i32,
    pub height: i32,
}

impl DisplayBuffer {
    pub fn step_render(&mut self, step_by: i32) {
        assert!(self.width > 0 && self.height > 0);

        assert!(self.memory.len() == self.height as usize * self.width as usize);
        for (i, pixel) in self.memory.iter_mut().enumerate() {
            assert!(i < i32::max_value() as usize);
            let x = i as i32 % self.width;
            let y = i as i32 / self.height;
            pixel.g = ((x ^ y) - self.current_offset) as u8;
        }

        self.current_offset += step_by;
    }
}

pub struct SoundBuffer {
    pub samples: Vec<i16>,
    pub sample_count: usize,
    pub t_sin: f32,
    pub volume: f32,
    pub sample_rate: u16,
}

impl SoundBuffer {
    fn render_sound(&mut self, tone_hz: u16) {
        let wave_period = f32::from(self.sample_rate) / f32::from(tone_hz);

        // TODO(sawlody) `2` is the number of channels - should be put in a variable
        for i in (0..self.sample_count * 2).step_by(2) {
            let sample_value = (self.t_sin.sin() * self.volume) as i16;

            self.samples[i] = sample_value;
            self.samples[i + 1] = sample_value;

            self.t_sin += 2.0 * f32::consts::PI * 1.0 / wave_period;
        }
    }
}

pub fn update_and_render(
    display_buffer: &mut DisplayBuffer,
    sound_buffer: &mut SoundBuffer,
    tone_hz: u16,
) {
    sound_buffer.render_sound(tone_hz);
    display_buffer.step_render(1);
}
