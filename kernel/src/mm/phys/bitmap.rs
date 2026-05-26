use crate::mm::layout::PAGE_SIZE;

/// Dense bitmap tracking frame availability (1 = allocated, 0 = free).
pub struct FrameBitmap {
    bits: &'static mut [u64],
    frame_count: u64,
}

impl FrameBitmap {
    pub fn new(storage: &'static mut [u64], frame_count: u64) -> Self {
        for word in storage.iter_mut() {
            *word = 0;
        }
        Self {
            bits: storage,
            frame_count,
        }
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn mark_all_used(&mut self) {
        for word in self.bits.iter_mut() {
            *word = u64::MAX;
        }
    }

    pub fn is_free(&self, frame: u64) -> bool {
        if frame >= self.frame_count {
            return false;
        }
        !self.test(frame)
    }

    pub fn is_used(&self, frame: u64) -> bool {
        self.test(frame)
    }

    pub fn set_used(&mut self, frame: u64) {
        self.set(frame, true);
    }

    pub fn set_free(&mut self, frame: u64) {
        self.set(frame, false);
    }

    pub fn reserve(&mut self, start: u64, count: u64) {
        for frame in start..start + count {
            self.set_used(frame);
        }
    }

    pub fn find_free(&self) -> Option<u64> {
        self.find_free_from(0)
    }

    pub fn find_free_from(&self, start: u64) -> Option<u64> {
        if start >= self.frame_count {
            return None;
        }
        let mut frame = start;
        while frame < self.frame_count {
            let word_idx = (frame / 64) as usize;
            let bit = frame % 64;
            let word = self.bits[word_idx];
            if word >> bit == 0 {
                // entire rest of word free
                let trailing = (!word).trailing_zeros() as u64;
                let candidate = frame + trailing - bit;
                if candidate < self.frame_count && !self.test(candidate) {
                    return Some(candidate);
                }
            }
            if !self.test(frame) {
                return Some(frame);
            }
            frame += 1;
        }
        None
    }

    pub fn count_free(&self) -> u64 {
        let full_words = (self.frame_count / 64) as usize;
        let mut free = 0u64;
        for word in &self.bits[..full_words] {
            free += word.count_zeros() as u64;
        }
        let tail = self.frame_count % 64;
        if tail != 0 {
            let mask = (1u64 << tail) - 1;
            free += (mask & !self.bits[full_words]).count_zeros() as u64;
        }
        free
    }

    fn test(&self, frame: u64) -> bool {
        let word_idx = (frame / 64) as usize;
        let bit = frame % 64;
        (self.bits[word_idx] >> bit) & 1 == 1
    }

    fn set(&mut self, frame: u64, used: bool) {
        let word_idx = (frame / 64) as usize;
        let bit = frame % 64;
        if used {
            self.bits[word_idx] |= 1 << bit;
        } else {
            self.bits[word_idx] &= !(1 << bit);
        }
    }
}

pub fn words_for_frames(frame_count: u64) -> usize {
    ((frame_count + 63) / 64) as usize
}

pub fn bytes_for_frames(frame_count: u64) -> u64 {
    words_for_frames(frame_count) as u64 * 8
}

pub fn phys_from_frame(frame: u64) -> u64 {
    frame * PAGE_SIZE
}

pub fn frame_from_phys(phys: u64) -> u64 {
    phys / PAGE_SIZE
}
