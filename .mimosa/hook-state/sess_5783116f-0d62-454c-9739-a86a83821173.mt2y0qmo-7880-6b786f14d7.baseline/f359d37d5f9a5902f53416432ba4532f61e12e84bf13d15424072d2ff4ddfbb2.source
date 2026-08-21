//! Issue #2746: agent_engine drain threads truncated stdout/stderr beyond 4KB.
//!
//! Bug: When `drain_done` was set, each drain thread performed a SINGLE
//! non-blocking read into a 4096-byte buffer and then broke, silently dropping
//! any residual pipe data larger than 4KB (normal-exit burst, or grandchild
//! residual output still buffered in the pipe, see #2440).
//!
//! Root cause: a fixed-size `tmp = [0u8; 4096]` read followed by `break`
//! captured at most one buffer worth of bytes even though more data was
//! available in the pipe.
//!
//! Fix: loop the final drain until WouldBlock/EOF so the entire residual
//! buffer is captured. The FD is non-blocking (#2541) so the loop never hangs.
//! The logic lives in `agent_engine::drain_nonblocking_remaining`, exercised
//! directly here.

#[cfg(test)]
mod tests {
    use crate::agent_engine::drain_nonblocking_remaining;
    use std::collections::VecDeque;
    use std::io::{Cursor, Read};

    /// A child exiting with a >4KB burst fills the pipe with e.g. 10_000 bytes.
    /// On Unix the drain loops to capture all bytes; on Windows (#3807) a
    /// single 4 KB read is the safe maximum to avoid indefinite thread hangs.
    #[test]
    fn regression_2746_drain_captures_more_than_4kb() {
        let data = vec![b'x'; 10_000];
        let mut reader = Cursor::new(data);
        let mut buf = Vec::new();
        drain_nonblocking_remaining("stdout", &mut reader, &mut buf);
        #[cfg(unix)]
        {
            assert_eq!(
                buf.len(),
                10_000,
                "final drain must capture ALL residual bytes, not just the first 4KB"
            );
        }
        #[cfg(not(unix))]
        {
            // Windows single-read fallback (#3807) — at most one 4 KB buffer.
            assert_eq!(buf.len(), 4096);
        }
        assert!(buf.iter().all(|&b| b == b'x'));
    }

    /// Boundary sizes around the 4096-byte read buffer must not truncate.
    /// On Windows (#3807) only the first 4 KB are read — sizes beyond that
    /// are not expected.
    #[test]
    fn regression_2746_drain_handles_exact_and_small() {
        for size in [0usize, 1, 4095, 4096, 4097, 8192, 12_345] {
            let data = vec![b'a'; size];
            let mut reader = Cursor::new(data);
            let mut buf = Vec::new();
            drain_nonblocking_remaining("stdout", &mut reader, &mut buf);
            #[cfg(unix)]
            {
                assert_eq!(buf.len(), size, "final drain truncated at size {size}");
            }
            #[cfg(not(unix))]
            {
                let expected = size.min(4096);
                assert_eq!(
                    buf.len(),
                    expected,
                    "final drain size mismatch at size {size}"
                );
            }
        }
    }

    /// Simulates a non-blocking pipe: first read returns a 4096-byte chunk,
    /// then WouldBlock (writer paused), and nothing further follows because the
    /// child has truly exited. The drain must consume the buffered chunk and
    /// then stop on WouldBlock without hanging or dropping buffered data.
    struct MockPipe {
        steps: VecDeque<Step>,
    }

    enum Step {
        Data(Vec<u8>),
        WouldBlock,
    }

    impl Read for MockPipe {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.steps.pop_front() {
                Some(Step::Data(d)) => {
                    let n = d.len().min(buf.len());
                    buf[..n].copy_from_slice(&d[..n]);
                    Ok(n)
                }
                Some(Step::WouldBlock) => Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "would block",
                )),
                None => Ok(0),
            }
        }
    }

    #[test]
    fn regression_2746_drain_stops_at_wouldblock_after_buffered_data() {
        let mut pipe = MockPipe {
            steps: VecDeque::from(vec![Step::Data(vec![b'y'; 4096]), Step::WouldBlock]),
        };
        let mut buf = Vec::new();
        drain_nonblocking_remaining("stdout", &mut pipe, &mut buf);
        assert_eq!(buf.len(), 4096);
    }
}
