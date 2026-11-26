//! Plays a GIF: a thread decodes frames one at a time, keeping two ready,
//! and starts over at the end. Each frame comes whole (the GIF's partial
//! frames drawn onto the picture so far), premultiplied, with its delay.

use std::io::Cursor;
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use holo_platform::waker;

/// Larger canvases are not played (a frame alone would be over 64 MB).
const MAX_PIXELS: usize = 4096 * 4096;

pub struct Frame {
    pub rgba: Vec<u8>,
    pub delay: Duration,
}

pub struct GifPlayer {
    rx: Receiver<Frame>,
    pub w: u32,
    pub h: u32,
}

pub(crate) fn open(bytes: &[u8]) -> Result<gif::Decoder<Cursor<&[u8]>>, gif::DecodingError> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    opts.read_info(Cursor::new(bytes))
}

impl GifPlayer {
    pub fn new(bytes: Vec<u8>) -> Option<GifPlayer> {
        let dec = open(&bytes).ok()?;
        let (w, h) = (dec.width() as u32, dec.height() as u32);
        if w == 0 || h == 0 || (w * h) as usize > MAX_PIXELS {
            return None;
        }
        drop(dec);
        let (tx, rx) = sync_channel::<Frame>(2);
        let bytes = Arc::new(bytes);
        thread::Builder::new()
            .name("gif".into())
            .spawn(move || {
                // Ends when the viewer drops the player (sending fails).
                loop {
                    let Ok(mut dec) = open(&bytes) else { return };
                    let mut canvas = vec![0u8; (w * h * 4) as usize];
                    let mut frames = 0;
                    while let Ok(Some(f)) = dec.read_next_frame() {
                        frames += 1;
                        let saved = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
                        draw(&mut canvas, w, f);
                        // Shown the way browsers do (and as "Save as video" saves it).
                        let delay = Duration::from_millis(gallery_core::gif_video::delay_ms(f.delay));
                        if tx.send(Frame { rgba: canvas.clone(), delay }).is_err() {
                            return;
                        }
                        waker::wake();
                        match f.dispose {
                            gif::DisposalMethod::Background => clear(&mut canvas, w, f),
                            gif::DisposalMethod::Previous => {
                                if let Some(s) = saved {
                                    canvas = s;
                                }
                            }
                            _ => {}
                        }
                    }
                    // A one-frame GIF is a still picture: nothing more to play.
                    if frames <= 1 {
                        return;
                    }
                }
            })
            .ok()?;
        Some(GifPlayer { rx, w, h })
    }

    /// The next frame, if it's ready.
    pub fn next(&self) -> Option<Frame> {
        self.rx.try_recv().ok()
    }
}

/// Opaque pixels of `f` over the canvas; see-through ones leave it be.
pub(crate) fn draw(canvas: &mut [u8], w: u32, f: &gif::Frame) {
    let (fl, ft, fw) = (f.left as usize, f.top as usize, f.width as usize);
    let cw = w as usize;
    let ch = canvas.len() / 4 / cw;
    for (row, line) in f.buffer.chunks_exact(fw * 4).enumerate() {
        let y = ft + row;
        if y >= ch {
            break;
        }
        for (col, px) in line.chunks_exact(4).enumerate() {
            let x = fl + col;
            if x >= cw || px[3] == 0 {
                continue;
            }
            let at = (y * cw + x) * 4;
            canvas[at..at + 4].copy_from_slice(px);
        }
    }
}

pub(crate) fn clear(canvas: &mut [u8], w: u32, f: &gif::Frame) {
    let cw = w as usize;
    let ch = canvas.len() / 4 / cw;
    for y in f.top as usize..(f.top as usize + f.height as usize).min(ch) {
        let from = (y * cw + (f.left as usize).min(cw)) * 4;
        let to = (y * cw + (f.left as usize + f.width as usize).min(cw)) * 4;
        canvas[from..to].fill(0);
    }
}
