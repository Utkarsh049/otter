use std::sync::Mutex;

pub struct SlotAllocator {
    slots: Mutex<Vec<bool>>,
}

impl SlotAllocator {
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: Mutex::new(vec![false; capacity]),
        }
    }

    pub fn allocate(&self) -> Option<usize> {
        let mut guard = self.slots.lock().unwrap();
        for (i, is_busy) in guard.iter_mut().enumerate() {
            if !*is_busy {
                *is_busy = true;
                return Some(i);
            }
        }
        None
    }

    pub fn release(&self, slot: usize) {
        let mut guard = self.slots.lock().unwrap();
        if slot < guard.len() {
            guard[slot] = false;
        }
    }
}
