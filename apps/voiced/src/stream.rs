//! CV3: the chunk player. The streaming tail hands each completed
//! sentence to this player the moment it lands in the journal; the
//! player synthesizes and plays the sentences *in order* on its own
//! thread while the tail keeps reading the still-running stream —
//! that overlap is what takes the brain's generation time out of
//! the turn's critical path.
//!
//! The player keeps the classic turn contract: the turn's `out_wav`
//! is still one file holding the whole spoken answer, now assembled
//! by concatenating the chunk WAVs. A failure in any chunk fails
//! the turn loudly (`tts_failed`), never silently skips a sentence;
//! sentences already played stay played — they are journaled, and
//! the journal says exactly how far speech got.
//!
//! The player owns the turn's ack watch: before the first chunk is
//! synthesized it settles the ack (which joins the watcher, so a
//! clip already playing finishes first) — a fired ack never
//! overlaps answer audio, exactly the CV0 rule, now kept on the
//! streaming path too.
//!
//! The player journals into its own journal (the turn journal is
//! single-threaded); `finish` merges those events back with their
//! trace ids preserved, the same pattern the request thread uses.

use std::sync::mpsc;
use std::thread::JoinHandle;

use voice_bridge::journal::Journal;
use voice_bridge::wav;

use crate::ack::AckWatch;
use crate::capture::JournalOut;
use crate::config::Config;
use crate::error::VoiceError;
use crate::tts;

/// Merge a helper thread's journal events back into the turn's
/// journal: payloads and trace correlation are preserved; seq and
/// timestamps are re-stamped by the journal of record.
pub fn merge_journal(journal: &mut JournalOut, events: Vec<trace_events::Event>) {
    for event in events {
        let trace = event.trace_id.unwrap_or_else(|| journal.new_trace());
        let _ = journal.emit(trace, event.span_id, event.parent_span_id, event.payload.clone());
    }
}

/// A journal a helper thread can own outright: created inside the
/// thread (the boxed writer is not `Send`, so it may not cross a
/// thread boundary in either direction) and drained into events
/// before the thread ends.
pub fn thread_journal() -> JournalOut {
    Journal::with_service(
        Box::new(Vec::<u8>::new()) as Box<dyn std::io::Write>,
        crate::SERVICE_NAME.to_string(),
    )
}

/// The chunk file for sentence `index` of a turn whose answer file
/// is `out_wav`: `answer.wav` → `answer.chunk0.wav`, …
pub fn chunk_path(out_wav: &str, index: usize) -> String {
    match out_wav.strip_suffix(".wav") {
        Some(stem) => format!("{stem}.chunk{index}.wav"),
        None => format!("{out_wav}.chunk{index}.wav"),
    }
}

pub struct ChunkPlayer {
    tx: Option<mpsc::Sender<String>>,
    handle: Option<JoinHandle<(Result<usize, VoiceError>, Vec<trace_events::Event>)>>,
}

impl ChunkPlayer {
    /// Start the player thread. The ack watch moves with it: the
    /// first chunk settles the ack before any synthesis begins.
    pub fn start(config: &Config, out_wav: &str, ack: Option<AckWatch>) -> ChunkPlayer {
        let (tx, rx) = mpsc::channel::<String>();
        let config = config.clone();
        let out_wav = out_wav.to_string();
        let handle = std::thread::spawn(
            move || -> (Result<usize, VoiceError>, Vec<trace_events::Event>) {
                let mut journal = thread_journal();
                let result = play_chunks(&config, &out_wav, ack, &rx, &mut journal);
                (result, journal.events())
            },
        );
        ChunkPlayer { tx: Some(tx), handle: Some(handle) }
    }

    /// Hand the next sentence to the player. Returns false if the
    /// player thread is already gone (its error surfaces at
    /// `finish`).
    pub fn push(&self, text: &str) -> bool {
        match &self.tx {
            Some(tx) => tx.send(text.to_string()).is_ok(),
            None => false,
        }
    }

    /// Close the queue, wait for the remaining chunks to be spoken,
    /// merge the player's journal into the turn journal, and return
    /// how many chunks were played.
    pub fn finish(mut self, journal: &mut JournalOut) -> Result<usize, VoiceError> {
        drop(self.tx.take());
        let handle = self.handle.take().expect("player handle");
        match handle.join() {
            Ok((result, player_events)) => {
                merge_journal(journal, player_events);
                result
            }
            Err(_) => Err(VoiceError::TtsFailed {
                engine: "chunk-player".to_string(),
                detail: "chunk player thread panicked".to_string(),
            }),
        }
    }
}

impl Drop for ChunkPlayer {
    fn drop(&mut self) {
        // Never leave a detached player speaking into the next turn.
        drop(self.tx.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn play_chunks(
    config: &Config,
    out_wav: &str,
    ack: Option<AckWatch>,
    rx: &mpsc::Receiver<String>,
    journal: &mut JournalOut,
) -> Result<usize, VoiceError> {
    let mut ack = ack;
    let mut paths: Vec<String> = Vec::new();
    let mut index = 0usize;
    while let Ok(text) = rx.recv() {
        // CV0 rule, kept: a fired ack clip finishes before the first
        // answer audio; settle joins the watcher to prove it.
        if let Some(watch) = ack.take() {
            watch.settle(journal);
        }
        let path = chunk_path(out_wav, index);
        tts::synthesize(config, &text, &path, journal)?;
        tts::play(config, &path, journal)?;
        paths.push(path);
        index += 1;
    }
    if !paths.is_empty() {
        // The turn's answer file is the chunks concatenated, in
        // order — the same PCM contract the whole-answer path made.
        let mut samples: Vec<u8> = Vec::new();
        let mut shape: Option<(u32, u16, u8)> = None;
        for path in &paths {
            let pcm = wav::read_pcm(path).map_err(|e| VoiceError::TtsFailed {
                engine: "chunk-player".to_string(),
                detail: format!("read chunk {path}: {e}"),
            })?;
            match shape {
                None => shape = Some((pcm.rate, pcm.channels, pcm.width)),
                Some((rate, channels, width)) => {
                    if (pcm.rate, pcm.channels, pcm.width) != (rate, channels, width) {
                        return Err(VoiceError::TtsFailed {
                            engine: "chunk-player".to_string(),
                            detail: format!("chunk {path} PCM shape differs from the first chunk"),
                        });
                    }
                }
            }
            samples.extend_from_slice(&pcm.samples);
        }
        let (rate, channels, width) = shape.expect("paths is non-empty");
        wav::write_pcm(out_wav, rate, width, channels, &samples).map_err(|e| VoiceError::TtsFailed {
            engine: "chunk-player".to_string(),
            detail: format!("write combined answer: {e}"),
        })?;
    }
    Ok(paths.len())
}
