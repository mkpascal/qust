// The plugins on the output thread: each enabled plugin is a stage, in the list's order. The mix
// comes in as interleaved stereo; each stage splits it into the plugin's channels, hands the
// plugin the parameter changes its editor made since the last block, runs it, and puts its
// output back. Everything a block needs is made beforehand: nothing here allocates or waits.

use super::objects::Changes;
use crate::ring::ParamRing;
use std::sync::Arc;
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_::*;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper};

/// The most frames a plugin is given at once (`maxSamplesPerBlock`).
pub const MAX_BLOCK: usize = 1024;

// A direction's buses: each bus's channels, the pointer arrays the plugin reads them through,
// and the bus descriptions that point at those.
struct Buses {
    channels: Vec<Vec<Vec<f32>>>,
    _pointers: Vec<Vec<*mut f32>>,
    buffers: Vec<AudioBusBuffers>,
}

impl Buses {
    fn new(counts: &[i32]) -> Buses {
        let mut channels: Vec<Vec<Vec<f32>>> = counts.iter().map(|count| (0..(*count).max(0)).map(|_| vec![0.0; MAX_BLOCK]).collect()).collect();
        let mut pointers: Vec<Vec<*mut f32>> = channels.iter_mut().map(|bus| bus.iter_mut().map(|channel| channel.as_mut_ptr()).collect()).collect();
        // The heap blocks don't move when the vectors do: the pointers stay good.
        let buffers = pointers
            .iter_mut()
            .map(|bus| AudioBusBuffers {
                numChannels: bus.len() as int32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 { channelBuffers32: if bus.is_empty() { std::ptr::null_mut() } else { bus.as_mut_ptr() } },
            })
            .collect();
        Buses { channels, _pointers: pointers, buffers }
    }

    fn reset_flags(&mut self) {
        for (buffer, channels) in self.buffers.iter_mut().zip(&self.channels) {
            buffer.numChannels = channels.len() as int32;
            buffer.silenceFlags = 0;
        }
    }
}

pub struct Stage {
    processor: ComPtr<IAudioProcessor>,
    inputs: Buses,
    outputs: Buses,
    input_changes: ComWrapper<Changes>,
    output_changes: ComWrapper<Changes>,
    to_audio: Arc<ParamRing>,
    from_audio: Arc<ParamRing>,
    context: ProcessContext,
}

impl Stage {
    pub fn new(processor: ComPtr<IAudioProcessor>, inputs: &[i32], outputs: &[i32], rate: u32, to_audio: Arc<ParamRing>, from_audio: Arc<ParamRing>) -> Stage {
        let mut context: ProcessContext = unsafe { std::mem::zeroed() };
        context.sampleRate = rate as f64;
        context.tempo = 120.0;
        context.timeSigNumerator = 4;
        context.timeSigDenominator = 4;
        context.state = (kPlaying | kTempoValid | kTimeSigValid | kContTimeValid | kProjectTimeMusicValid) as uint32;
        Stage {
            processor,
            inputs: Buses::new(inputs),
            outputs: Buses::new(outputs),
            input_changes: ComWrapper::new(Changes::new()),
            output_changes: ComWrapper::new(Changes::new()),
            to_audio,
            from_audio,
            context,
        }
    }

    // One block of at most MAX_BLOCK frames, in place.
    fn run(&mut self, stereo: &mut [f32]) {
        let frames = stereo.len() / 2;
        if let Some(main) = self.inputs.channels.first_mut() {
            match main.len() {
                0 => {}
                1 => {
                    for (i, frame) in stereo.as_chunks::<2>().0.iter().enumerate() {
                        main[0][i] = (frame[0] + frame[1]) * 0.5;
                    }
                }
                _ => {
                    let (left, rest) = main.split_at_mut(1);
                    for (i, frame) in stereo.as_chunks::<2>().0.iter().enumerate() {
                        left[0][i] = frame[0];
                        rest[0][i] = frame[1];
                    }
                    for channel in &mut rest[1..] {
                        channel[..frames].fill(0.0);
                    }
                }
            }
        }
        // Side chains and other inputs hear nothing.
        for bus in self.inputs.channels.iter_mut().skip(1) {
            for channel in bus {
                channel[..frames].fill(0.0);
            }
        }
        self.inputs.reset_flags();
        self.outputs.reset_flags();

        self.input_changes.clear();
        while let Some((id, value)) = self.to_audio.pop() {
            self.input_changes.set(id, value);
        }
        self.output_changes.clear();

        let seconds = self.context.projectTimeSamples as f64 / self.context.sampleRate.max(1.0);
        self.context.projectTimeMusic = seconds * self.context.tempo / 60.0;
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            numSamples: frames as int32,
            numInputs: self.inputs.buffers.len() as int32,
            numOutputs: self.outputs.buffers.len() as int32,
            inputs: if self.inputs.buffers.is_empty() { std::ptr::null_mut() } else { self.inputs.buffers.as_mut_ptr() },
            outputs: if self.outputs.buffers.is_empty() { std::ptr::null_mut() } else { self.outputs.buffers.as_mut_ptr() },
            inputParameterChanges: self.input_changes.as_com_ref::<IParameterChanges>().map_or(std::ptr::null_mut(), |changes| changes.as_ptr()),
            outputParameterChanges: self.output_changes.as_com_ref::<IParameterChanges>().map_or(std::ptr::null_mut(), |changes| changes.as_ptr()),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: &mut self.context,
        };
        let result = unsafe { self.processor.process(&mut data) };
        self.context.projectTimeSamples += frames as i64;
        self.context.continousTimeSamples += frames as i64;

        // What the plugin reported (meters, gain reduction…) goes to its editor.
        for queue in self.output_changes.filled() {
            if let Some(value) = queue.last() {
                self.from_audio.push(queue.id(), value);
            }
        }
        // A plugin that refused the block leaves the sound as it was.
        if result != kResultOk {
            return;
        }
        let Some(main) = self.outputs.channels.first() else { return };
        match main.len() {
            0 => {}
            1 => {
                for (i, frame) in stereo.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    frame[0] = main[0][i];
                    frame[1] = main[0][i];
                }
            }
            _ => {
                for (i, frame) in stereo.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    frame[0] = main[0][i];
                    frame[1] = main[1][i];
                }
            }
        }
    }
}

/// The enabled plugins in order: what the engine runs on its mix.
pub struct Chain {
    pub stages: Vec<Stage>,
}

// The COM pointers and buffers move to the output thread once, with the chain, and are used
// there only (the plugin thread builds a new chain rather than touch this one).
unsafe impl Send for Chain {}

impl qust_audio::Effect for Chain {
    fn process(&mut self, stereo: &mut [f32]) {
        for block in stereo.chunks_mut(MAX_BLOCK * 2) {
            for stage in &mut self.stages {
                stage.run(block);
            }
        }
    }
}
