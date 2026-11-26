//! One thread that decodes pictures for the viewer through Java: whole
//! pictures at screen size, zoomed-in parts at full detail, GIF bytes and
//! camera details. The viewer says what it wants most; a newer wish list
//! replaces the old one, so swiping fast never queues up stale work.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use android_activity::AndroidApp;
use holo_platform::jni_util::with_activity;
use holo_platform::waker;
use jni::objects::{JByteArray, JObject, JString, JValue};
use jni::Env;
use jni::{jni_sig, jni_str};

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// The whole picture, no bigger than `max` either way round.
    Full { id: i64, orientation: u16, max: (u32, u32) },
    /// `rect` (x, y, w, h in upright full-size pixels) at 1/`sample` detail.
    Region { id: i64, orientation: u16, rect: (u32, u32, u32, u32), sample: u32 },
    Gif { id: i64 },
    Details { id: i64 },
}

pub enum Done {
    Full { id: i64, w: u32, h: u32, full: (u32, u32), rgba: Vec<u8> },
    Region { id: i64, rect: (u32, u32, u32, u32), w: u32, h: u32, rgba: Vec<u8> },
    Gif { id: i64, bytes: Vec<u8> },
    Details { id: i64, fields: [String; 5] },
    Failed(Job),
}

#[derive(Default)]
struct State {
    wanted: Vec<Job>,
    /// Being done now, or done and not yet taken: not to be started again.
    busy: Vec<Job>,
    done: VecDeque<Done>,
    app: Option<AndroidApp>,
}

pub struct Jobs {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl Jobs {
    pub fn new(app: &AndroidApp) -> Jobs {
        let shared: Arc<(Mutex<State>, Condvar)> = Arc::default();
        shared.0.lock().unwrap().app = Some(app.clone());
        let s = shared.clone();
        thread::Builder::new().name("viewer".into()).spawn(move || work(s)).expect("viewer thread");
        Jobs { shared }
    }

    /// What's wanted now, most wanted first. Replaces the last list.
    pub fn want(&self, jobs: Vec<Job>) {
        let (lock, cv) = &*self.shared;
        let mut s = lock.lock().unwrap();
        let mut jobs = jobs;
        jobs.retain(|j| !s.busy.contains(j));
        if s.wanted != jobs {
            s.wanted = jobs;
            cv.notify_all();
        }
    }

    /// Finished work is waiting to be taken.
    pub fn has_done(&self) -> bool {
        !self.shared.0.lock().unwrap().done.is_empty()
    }

    pub fn take(&self) -> Vec<Done> {
        let mut s = self.shared.0.lock().unwrap();
        let running = s.busy.len().saturating_sub(s.done.len());
        // The finished ones are no longer busy; the running one (first) still is.
        s.busy.truncate(running);
        s.done.drain(..).collect()
    }
}

fn work(shared: Arc<(Mutex<State>, Condvar)>) {
    let (lock, cv) = &*shared;
    loop {
        let (job, app) = {
            let mut s = lock.lock().unwrap();
            loop {
                if !s.wanted.is_empty() && s.app.is_some() {
                    let job = s.wanted.remove(0);
                    s.busy.insert(0, job.clone());
                    break (job, s.app.clone().unwrap());
                }
                s = cv.wait(s).unwrap();
            }
        };
        let t = Instant::now();
        let done = run(&app, &job).unwrap_or(Done::Failed(job.clone()));
        log::debug!("viewer job {job:?}: {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
        lock.lock().unwrap().done.push_back(done);
        waker::wake();
    }
}

/// A picture from Java: 16-byte header (w, h, full w, full h) then RGBA.
fn unpack(raw: Vec<u8>) -> Option<(u32, u32, (u32, u32), Vec<u8>)> {
    if raw.len() < 16 {
        return None;
    }
    let int = |i: usize| u32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
    let (w, h, fw, fh) = (int(0), int(4), int(8), int(12));
    if raw.len() < 16 + (w * h * 4) as usize {
        return None;
    }
    let mut raw = raw;
    raw.drain(..16);
    Some((w, h, (fw, fh), raw))
}

/// A Java byte[] (or null) as bytes.
fn array_bytes(env: &mut Env, obj: JObject) -> jni::errors::Result<Option<Vec<u8>>> {
    if obj.is_null() {
        return Ok(None);
    }
    let array = env.cast_local::<JByteArray>(obj)?;
    env.convert_byte_array(&array).map(Some)
}

fn logged<T>(what: &str, r: jni::errors::Result<Option<T>>) -> Option<T> {
    r.unwrap_or_else(|e| {
        log::warn!("{what} failed: {e}");
        None
    })
}

fn run(app: &AndroidApp, job: &Job) -> Option<Done> {
    match *job {
        Job::Full { id, orientation, max } => {
            let raw = logged(
                "decodePicture",
                with_activity(app, |env, activity| {
                    let args = [JValue::Long(id), JValue::Int(orientation as i32), JValue::Int(max.0 as i32), JValue::Int(max.1 as i32)];
                    let obj = env.call_method(activity, jni_str!("decodePicture"), jni_sig!("(JIII)[B"), &args)?.l()?;
                    array_bytes(env, obj)
                }),
            )?;
            let (w, h, full, rgba) = unpack(raw)?;
            Some(Done::Full { id, w, h, full, rgba })
        }
        Job::Region { id, orientation, rect, sample } => {
            let raw = logged(
                "decodeRegion",
                with_activity(app, |env, activity| {
                    let args = [JValue::Long(id), JValue::Int(orientation as i32), JValue::Int(rect.0 as i32), JValue::Int(rect.1 as i32), JValue::Int(rect.2 as i32), JValue::Int(rect.3 as i32), JValue::Int(sample as i32)];
                    let obj = env.call_method(activity, jni_str!("decodeRegion"), jni_sig!("(JIIIIII)[B"), &args)?.l()?;
                    array_bytes(env, obj)
                }),
            )?;
            let (w, h, _, rgba) = unpack(raw)?;
            Some(Done::Region { id, rect, w, h, rgba })
        }
        Job::Gif { id } => {
            let bytes = logged(
                "readAll",
                with_activity(app, |env, activity| {
                    let obj = env.call_method(activity, jni_str!("readAll"), jni_sig!("(J)[B"), &[JValue::Long(id)])?.l()?;
                    array_bytes(env, obj)
                }),
            )?;
            Some(Done::Gif { id, bytes })
        }
        Job::Details { id } => {
            let text = logged(
                "pictureDetails",
                with_activity(app, |env, activity| {
                    let obj = env.call_method(activity, jni_str!("pictureDetails"), jni_sig!("(J)Ljava/lang/String;"), &[JValue::Long(id)])?.l()?;
                    if obj.is_null() {
                        return Ok(None);
                    }
                    let s = env.cast_local::<JString>(obj)?;
                    Ok(Some(String::from(s.mutf8_chars(env)?)))
                }),
            )?;
            let mut fields: [String; 5] = Default::default();
            for (slot, part) in fields.iter_mut().zip(text.split('\n')) {
                *slot = part.to_string();
            }
            Some(Done::Details { id, fields })
        }
    }
}
