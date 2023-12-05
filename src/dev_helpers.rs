#[cfg(test)]
mod funky {
    use std::convert::TryInto;

    use std::io::Read;
    use std::io::Result as IOResult;

    #[derive(Debug, Default)]
    pub struct FunnyRead {
        counter: usize
    }
    impl Read for FunnyRead {
        fn read(&mut self, buf: &mut [u8]) -> IOResult<usize> {
            let mut actual_count = 0;
            for byte in buf.into_iter().take(3) {
                actual_count += 1;
                *byte = (self.counter%256).try_into().unwrap();
                self.counter += 1;
            }
            Ok(actual_count)
        }
    }
}

#[cfg(test)]
pub use self::funky::*;