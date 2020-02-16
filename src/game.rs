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

pub fn update_and_render(display_buffer: &mut DisplayBuffer) {
    display_buffer.step_render(1);
}
