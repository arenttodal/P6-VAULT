//! One exclusive worker thread owns the [`Device`] and serializes every MIDI
//! transaction. Auditions are latest-only: a pending audition is replaced, never queued.

use super::{Device, DeviceError};
use crate::protocol::payload::Payload;
use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::JoinHandle;

type Job = Box<dyn FnOnce(&mut Device) + Send>;

#[derive(Debug, Clone)]
pub struct AuditionRequest {
    pub id: u64,
    pub payload: Payload,
    pub label: String,
}

#[derive(Debug, Clone)]
pub enum AuditionOutcome {
    Sent { id: u64, label: String },
    Failed { id: u64, label: String, error: DeviceError },
}

struct State {
    jobs: VecDeque<Job>,
    audition: Option<AuditionRequest>,
    last_sent_hash: Option<String>,
    blocked: bool,
    stop: bool,
}

pub struct DeviceActor {
    shared: Arc<(Mutex<State>, Condvar)>,
    handle: Option<JoinHandle<()>>,
    epoch: u64,
}

impl DeviceActor {
    pub fn spawn(mut device: Device, on_audition: impl Fn(AuditionOutcome) + Send + 'static) -> Self {
        let epoch = device.epoch();
        let shared = Arc::new((
            Mutex::new(State { jobs: VecDeque::new(), audition: None, last_sent_hash: None, blocked: false, stop: false }),
            Condvar::new(),
        ));
        let s2 = shared.clone();
        let handle = std::thread::Builder::new()
            .name("p6-midi-actor".into())
            .spawn(move || loop {
                let (job, aud) = {
                    let (m, cv) = &*s2;
                    let mut st = m.lock().unwrap();
                    loop {
                        if st.stop {
                            let _ = device.release_notes();
                            return;
                        }
                        if let Some(j) = st.jobs.pop_front() {
                            break (Some(j), None);
                        }
                        if !st.blocked {
                            if let Some(a) = st.audition.take() {
                                break (None, Some(a));
                            }
                        }
                        st = cv.wait(st).unwrap();
                    }
                };
                if let Some(j) = job {
                    j(&mut device);
                } else if let Some(a) = aud {
                    let r = device.load_edit_buffer(&a.payload);
                    let (m, _) = &*s2;
                    if r.is_ok() {
                        m.lock().unwrap().last_sent_hash = Some(a.payload.exact_hash());
                    }
                    on_audition(match r {
                        Ok(()) => AuditionOutcome::Sent { id: a.id, label: a.label },
                        Err(error) => AuditionOutcome::Failed { id: a.id, label: a.label, error },
                    });
                }
            })
            .expect("spawn midi actor");
        Self { shared, handle: Some(handle), epoch }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Run a transaction exclusively on the device and wait for its result.
    pub fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut Device) -> R + Send + 'static) -> R {
        let (tx, rx) = mpsc::channel();
        self.submit(move |d| {
            let _ = tx.send(f(d));
        });
        rx.recv().expect("midi actor stopped")
    }

    pub fn submit(&self, f: impl FnOnce(&mut Device) + Send + 'static) {
        let (m, cv) = &*self.shared;
        m.lock().unwrap().jobs.push_back(Box::new(f));
        cv.notify_all();
    }

    /// Latest-only audition. `force` reloads even if the same payload was last sent.
    /// Returns false if auditions are currently blocked.
    pub fn audition(&self, req: AuditionRequest, force: bool) -> bool {
        let (m, cv) = &*self.shared;
        let mut st = m.lock().unwrap();
        if st.blocked {
            return false;
        }
        if !force && st.audition.is_none() && st.last_sent_hash.as_deref() == Some(&req.payload.exact_hash()) {
            return true;
        }
        st.audition = Some(req);
        cv.notify_all();
        true
    }

    /// Block (and purge) auditions during bank reads, write preparation, writes and recovery.
    pub fn set_audition_blocked(&self, blocked: bool) {
        let (m, cv) = &*self.shared;
        let mut st = m.lock().unwrap();
        st.blocked = blocked;
        st.audition = None;
        cv.notify_all();
    }

    pub fn audition_blocked(&self) -> bool {
        self.shared.0.lock().unwrap().blocked
    }
}

impl Drop for DeviceActor {
    fn drop(&mut self) {
        {
            let (m, cv) = &*self.shared;
            let mut st = m.lock().unwrap();
            st.stop = true;
            st.audition = None;
            cv.notify_all();
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
