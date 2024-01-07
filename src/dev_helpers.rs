#[cfg(test)]
mod funky {
    use std::cmp::min;
    use std::convert::TryInto;

    use std::io::ErrorKind;
    use std::io::Read;
    use std::io::Result as IOResult;

    #[derive(Debug, Default)]
    pub struct FunnyRead {
        counter: usize,
    }
    impl Read for FunnyRead {
        fn read(&mut self, buf: &mut [u8]) -> IOResult<usize> {
            let mut actual_count = 0;
            for byte in buf.into_iter().take(3) {
                actual_count += 1;
                *byte = (self.counter % 256).try_into().unwrap();
                self.counter += 1;
            }
            Ok(actual_count)
        }
    }

    /*
     * Scarlet
     * Compiler
     * on Crabby Patrol!!!!
     */
    #[derive(Debug, Default)]
    pub struct IceCubeRead {
        state: usize,
    }
    impl Read for IceCubeRead {
        fn read(&mut self, buf: &mut [u8]) -> IOResult<usize> {
            let retval = match self.state {
                0 | 2 => {
                    if buf.len() >= 1 {
                        buf[0] = 9;
                    }
                    if buf.len() >= 2 {
                        buf[1] = 99;
                    }
                    if buf.len() >= 3 {
                        buf[2] = 0x99;
                    }
                    Ok(min(buf.len(), 3))
                }
                1 => Err(ErrorKind::Interrupted.into()),
                3 => Err(ErrorKind::Other.into()),
                4 => Ok(0),
                _ => unreachable!(),
            };
            self.state = (self.state + 1) % 5;
            retval
        }
    }

    // Yukari's way to annoy Reimu by imposing read boundaries
    #[derive(Debug, Default)]
    pub struct TruncatedRead {
        data_vec: Vec<u8>,
        state: usize,
    }
    impl Read for TruncatedRead {
        fn read(&mut self, buf: &mut [u8]) -> IOResult<usize> {
            let retval = match self.state {
                0..=1 => {
                    if self.data_vec.len() == 0 {
                        self.data_vec = Vec::from(b"reimu");
                    }
                    let copy_section: Vec<_> = self
                        .data_vec
                        .drain(..min(buf.len(), self.data_vec.len()))
                        .collect();
                    buf[..copy_section.len()].copy_from_slice(&copy_section);
                    Ok(copy_section.len())
                }
                2 => Err(ErrorKind::Other.into()),
                3 => Ok(0),
                _ => unreachable!(),
            };
            self.state = (self.state + 1) % 4;
            retval
        }
    }
}

#[cfg(test)]
pub use self::funky::*;
