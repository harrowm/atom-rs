//! Getting the Atom Software Archive in the background.
//!
//! Downloading takes a few seconds (and the first time about 16 MB), which
//! must not freeze the window. The work runs on its own thread and sends what
//! it is doing back through a channel; the window collects it each frame.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

/// What the background thread reports.
#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    /// A progress message to show ("Downloading archive V14BETA2...").
    Said(String),
    /// Finished: the folder to use as the SD card, or what went wrong.
    Done(Result<PathBuf, String>),
}

pub struct ArchiveJob {
    receiver: Receiver<Progress>,
}

impl ArchiveJob {
    /// Run `work` on a new thread. It is given a function to report progress
    /// with, and returns the card folder.
    pub fn spawn(
        work: impl FnOnce(&mut dyn FnMut(&str)) -> Result<PathBuf, String> + Send + 'static,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = work(&mut |message| {
                // If the window has gone, nobody is listening: that is fine.
                let _ = sender.send(Progress::Said(message.to_string()));
            });
            let _ = sender.send(Progress::Done(result));
        });
        ArchiveJob { receiver }
    }

    /// Download (or update) the standard archive in `~/.cache/atom-rs/archive`.
    pub fn start_standard() -> Self {
        Self::spawn(|say| {
            let cache = crate::archive::cache_folder()?;
            crate::archive::prepare(&cache, &mut |message| say(message))
        })
    }

    /// Everything reported since the last call, oldest first.
    pub fn poll(&mut self) -> Vec<Progress> {
        let mut reports = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(report) => reports.push(report),
                Err(TryRecvError::Empty) => return reports,
                Err(TryRecvError::Disconnected) => {
                    // The thread ended without saying it was done (it panicked).
                    if !matches!(reports.last(), Some(Progress::Done(_))) {
                        reports.push(Progress::Done(Err(
                            "the download stopped unexpectedly".to_string()
                        )));
                    }
                    return reports;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Collect reports until the job says it is done (or a few seconds pass).
    fn wait_for_done(job: &mut ArchiveJob) -> Vec<Progress> {
        let mut all = Vec::new();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            all.extend(job.poll());
            if matches!(all.last(), Some(Progress::Done(_))) {
                return all;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("the job never finished: {all:?}");
    }

    #[test]
    fn progress_arrives_in_order_then_the_folder() {
        let mut job = ArchiveJob::spawn(|say| {
            say("Checking...");
            say("Downloading...");
            Ok(PathBuf::from("/cache/files"))
        });
        assert_eq!(
            wait_for_done(&mut job),
            vec![
                Progress::Said("Checking...".into()),
                Progress::Said("Downloading...".into()),
                Progress::Done(Ok("/cache/files".into())),
            ]
        );
    }

    #[test]
    fn a_failure_is_reported_not_lost() {
        let mut job = ArchiveJob::spawn(|_| Err("no network".to_string()));
        assert_eq!(
            wait_for_done(&mut job),
            vec![Progress::Done(Err("no network".into()))]
        );
    }

    #[test]
    fn a_thread_that_panics_still_ends_the_job() {
        let mut job = ArchiveJob::spawn(|_| panic!("boom"));
        let reports = wait_for_done(&mut job);
        assert!(matches!(reports.last(), Some(Progress::Done(Err(_)))));
    }

    #[test]
    fn polling_with_nothing_new_returns_nothing() {
        let mut job = ArchiveJob::spawn(|_| {
            thread::sleep(Duration::from_millis(200));
            Ok(PathBuf::new())
        });
        assert!(job.poll().is_empty());
    }
}
