// One running plugin: its processor (component) and its controller, connected to each other and
// to the host, set up for the engine's rate, its saved state restored, active and processing.
// Made and dropped on the plugin thread, which is the only one that calls it outside `process`.

use super::chain::{Stage, MAX_BLOCK};
use super::module::Module;
use super::objects::{Handler, HostApp, Stream};
use crate::ring::ParamRing;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use vst3::Steinberg::Vst::BusDirections_::*;
use vst3::Steinberg::Vst::MediaTypes_::*;
use vst3::Steinberg::Vst::RestartFlags_::*;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper};

pub const NO_AUDIO_IN: &str = "This plugin doesn't take audio in, so it can't process what Qust plays.";
pub const NO_FLOAT: &str = "This plugin can't process 32-bit audio.";
pub const NOT_STARTED: &str = "The plugin couldn't start.";

// Changes waiting for each direction: a few blocks' worth of a knob being dragged.
const RING: usize = 1024;

pub struct Instance {
    pub component: ComPtr<IComponent>,
    pub processor: ComPtr<IAudioProcessor>,
    pub controller: Option<ComPtr<IEditController>>,
    // The controller is an object of its own (else it is the component).
    separate: bool,
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    pub handler: ComWrapper<Handler>,
    to_audio: Arc<ParamRing>,
    from_audio: Arc<ParamRing>,
    inputs: Vec<i32>,
    outputs: Vec<i32>,
    rate: u32,
    active: bool,
    _module: Rc<Module>,
}

fn check(result: tresult, error: &str) -> Result<(), String> {
    if result == kResultOk {
        Ok(())
    } else {
        Err(error.to_string())
    }
}

unsafe fn stream_from(bytes: &[u8]) -> ComWrapper<Stream> {
    ComWrapper::new(Stream::with(bytes.to_vec()))
}

unsafe fn stream_ptr(stream: &ComWrapper<Stream>) -> *mut IBStream {
    stream.as_com_ref::<IBStream>().map_or(std::ptr::null_mut(), |stream| stream.as_ptr())
}

impl Instance {
    /// `state`: (the processor's, the controller's), as saved.
    pub fn create(module: Rc<Module>, host: &ComWrapper<HostApp>, rate: u32, state: Option<(Vec<u8>, Vec<u8>)>) -> Result<Instance, String> {
        unsafe {
            let context = host.as_com_ref::<IHostApplication>().map_or(std::ptr::null_mut(), |host| host.as_ptr() as *mut FUnknown);
            let cid = module.class.cid;
            let component: ComPtr<IComponent> = module.create(&cid).ok_or(NOT_STARTED)?;
            check(component.initialize(context), NOT_STARTED)?;
            let Some(processor) = component.cast::<IAudioProcessor>() else {
                component.terminate();
                return Err(super::module::NO_EFFECT.into());
            };
            let to_audio = Arc::new(ParamRing::new(RING));
            let handler = ComWrapper::new(Handler::new(to_audio.clone()));
            let mut instance = Instance {
                processor,
                component,
                controller: None,
                separate: false,
                connection: None,
                handler,
                to_audio,
                from_audio: Arc::new(ParamRing::new(RING)),
                inputs: Vec::new(),
                outputs: Vec::new(),
                rate,
                active: false,
                _module: module.clone(),
            };
            // From here on, dropping `instance` undoes what was done.
            instance.controller = instance.component.cast::<IEditController>();
            if instance.controller.is_none() {
                let mut controller_cid: TUID = [0; 16];
                if instance.component.getControllerClassId(&mut controller_cid) == kResultOk && controller_cid != [0; 16] {
                    if let Some(controller) = module.create::<IEditController>(&controller_cid) {
                        if controller.initialize(context) == kResultOk {
                            instance.controller = Some(controller);
                            instance.separate = true;
                        }
                    }
                }
            }
            if let Some(controller) = &instance.controller {
                if let Some(handler) = instance.handler.as_com_ref::<IComponentHandler>() {
                    controller.setComponentHandler(handler.as_ptr());
                }
                if instance.separate {
                    if let (Some(a), Some(b)) = (instance.component.cast::<IConnectionPoint>(), controller.cast::<IConnectionPoint>()) {
                        a.connect(b.as_ptr());
                        b.connect(a.as_ptr());
                        instance.connection = Some((a, b));
                    }
                }
            }
            instance.arrange()?;
            instance.restore(state);
            instance.activate()?;
            Ok(instance)
        }
    }

    // Stereo on the main buses where the plugin takes it; the others as they are, and off.
    unsafe fn arrange(&mut self) -> Result<(), String> {
        let component = &self.component;
        let processor = &self.processor;
        let ins = component.getBusCount(kAudio as MediaType, kInput as BusDirection).max(0);
        let outs = component.getBusCount(kAudio as MediaType, kOutput as BusDirection).max(0);
        if ins == 0 {
            return Err(NO_AUDIO_IN.into());
        }
        if outs == 0 {
            return Err(super::module::NO_EFFECT.into());
        }
        if processor.canProcessSampleSize(SymbolicSampleSizes_::kSample32 as int32) != kResultOk {
            return Err(NO_FLOAT.into());
        }
        let current = |direction: BusDirection, count: i32| -> Vec<SpeakerArrangement> {
            (0..count)
                .map(|index| {
                    let mut arrangement: SpeakerArrangement = 0;
                    processor.getBusArrangement(direction, index, &mut arrangement);
                    arrangement
                })
                .collect()
        };
        let mut in_arrangements = current(kInput as BusDirection, ins);
        let mut out_arrangements = current(kOutput as BusDirection, outs);
        let stereo = SpeakerArr::kStereo;
        in_arrangements[0] = stereo;
        out_arrangements[0] = stereo;
        // Refused: it keeps its own (mono, or more channels), which the chain maps.
        processor.setBusArrangements(in_arrangements.as_mut_ptr(), ins, out_arrangements.as_mut_ptr(), outs);
        let channels = |direction: BusDirection, count: i32| -> Vec<i32> {
            (0..count)
                .map(|index| {
                    let mut info: BusInfo = std::mem::zeroed();
                    if component.getBusInfo(kAudio as MediaType, direction, index, &mut info) == kResultOk {
                        info.channelCount.max(0)
                    } else {
                        0
                    }
                })
                .collect()
        };
        self.inputs = channels(kInput as BusDirection, ins);
        self.outputs = channels(kOutput as BusDirection, outs);
        for (direction, count) in [(kInput as BusDirection, ins), (kOutput as BusDirection, outs)] {
            for index in 0..count {
                component.activateBus(kAudio as MediaType, direction, index, (index == 0) as TBool);
            }
        }
        Ok(())
    }

    // The saved state, or the plugin's own defaults shown to its controller.
    unsafe fn restore(&self, state: Option<(Vec<u8>, Vec<u8>)>) {
        match state {
            Some((component_state, controller_state)) if !component_state.is_empty() => {
                let stream = stream_from(&component_state);
                self.component.setState(stream_ptr(&stream));
                if let Some(controller) = &self.controller {
                    let stream = stream_from(&component_state);
                    controller.setComponentState(stream_ptr(&stream));
                    if !controller_state.is_empty() {
                        let stream = stream_from(&controller_state);
                        controller.setState(stream_ptr(&stream));
                    }
                }
            }
            _ => {
                if let Some(controller) = &self.controller {
                    let stream = ComWrapper::new(Stream::default());
                    if self.component.getState(stream_ptr(&stream)) == kResultOk {
                        stream.rewind();
                        controller.setComponentState(stream_ptr(&stream));
                    }
                }
            }
        }
    }

    unsafe fn activate(&mut self) -> Result<(), String> {
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: MAX_BLOCK as int32,
            sampleRate: self.rate as f64,
        };
        check(self.processor.setupProcessing(&mut setup), NOT_STARTED)?;
        check(self.component.setActive(1), NOT_STARTED)?;
        self.active = true;
        // Some plugins answer kNotImplemented: processing goes ahead all the same.
        self.processor.setProcessing(1);
        Ok(())
    }

    unsafe fn deactivate(&mut self) {
        if self.active {
            self.processor.setProcessing(0);
            self.component.setActive(0);
            self.active = false;
        }
    }

    /// The stage that runs it, for a new chain.
    pub fn stage(&self) -> Stage {
        Stage::new(Clone::clone(&self.processor), &self.inputs, &self.outputs, self.rate, self.to_audio.clone(), self.from_audio.clone())
    }

    /// Its settings now: (the processor's state, the controller's).
    pub fn save(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        unsafe {
            let stream = ComWrapper::new(Stream::default());
            if self.component.getState(stream_ptr(&stream)) != kResultOk {
                return None;
            }
            let component = stream.bytes();
            let mut controller_state = Vec::new();
            if let (Some(controller), true) = (&self.controller, self.separate) {
                let stream = ComWrapper::new(Stream::default());
                if controller.getState(stream_ptr(&stream)) == kResultOk {
                    controller_state = stream.bytes();
                }
            }
            Some((component, controller_state))
        }
    }

    /// What the processor reported, shown in the editor.
    pub fn deliver_output(&self) {
        let Some(controller) = &self.controller else {
            while self.from_audio.pop().is_some() {}
            return;
        };
        while let Some((id, value)) = self.from_audio.pop() {
            unsafe {
                controller.setParamNormalized(id, value);
            }
        }
    }

    /// Whether the plugin asked to be restarted (new latency, new buses, reload): the caller takes
    /// it out of the chain, calls [`Instance::restart`], and puts it back.
    pub fn wants_restart(&self) -> bool {
        let flags = self.handler.restart.swap(0, Ordering::AcqRel);
        flags & (kReloadComponent | kIoChanged | kLatencyChanged) != 0
    }

    /// Off and on again, with its buses read anew. Only while out of the chain.
    pub fn restart(&mut self) -> Result<(), String> {
        unsafe {
            self.deactivate();
            self.arrange()?;
            self.activate()
        }
    }

    /// Edits since the last save (and forgets them).
    pub fn take_changed(&self) -> bool {
        self.handler.changed.swap(false, Ordering::AcqRel)
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            self.deactivate();
            if let Some((a, b)) = self.connection.take() {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(controller) = self.controller.take() {
                controller.setComponentHandler(std::ptr::null_mut());
                if self.separate {
                    controller.terminate();
                }
            }
            self.component.terminate();
        }
    }
}
