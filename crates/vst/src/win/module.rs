// A plugin's binary, loaded: its factory and the effect class Qust uses from it.
//
// Binaries stay loaded until Qust exits once loaded (as most hosts do): unloading code whose
// threads or callbacks may still be around is how hosts crash, and a binary costs little next
// to its instances, which do go with the engine.

use super::objects::{text, HostApp};
use std::ffi::c_void;
use std::path::Path;
use vst3::Steinberg::Vst::IHostApplication;
use vst3::Steinberg::*;
use vst3::{ComPtr, ComWrapper, Interface};
use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH};

pub const NO_EFFECT: &str = "This plugin has no audio effect in it.";
pub const INSTRUMENT: &str = "This is an instrument, not an effect: it makes sound instead of processing it.";

// "Audio Module Class": kVstAudioEffectClass, the processors (effects and instruments alike).
const AUDIO_CLASS: &str = "Audio Module Class";

#[derive(Clone)]
pub struct ClassInfo {
    pub cid: TUID,
    pub name: String,
    pub vendor: String,
}

pub struct Module {
    _library: HMODULE,
    pub factory: ComPtr<IPluginFactory>,
    pub class: ClassInfo,
}

type InitDll = unsafe extern "system" fn() -> bool;
type GetPluginFactory = unsafe extern "system" fn() -> *mut IPluginFactory;

impl Module {
    pub fn load(binary: &Path, host: &ComWrapper<HostApp>) -> Result<Module, String> {
        crate::bundle::check_binary(binary)?;
        let path: Vec<u16> = binary.as_os_str().encode_wide_terminated();
        unsafe {
            // Altered search path: the DLLs it depends on are found next to it.
            let library = LoadLibraryExW(PCWSTR(path.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH).map_err(|error| format!("Couldn't load the plugin: {}", error.message().trim_end_matches(['\r', '\n', '.'])))?;
            if let Some(init) = GetProcAddress(library, PCSTR(c"InitDll".as_ptr() as *const u8)) {
                let init: InitDll = std::mem::transmute(init);
                if !init() {
                    return Err("The plugin couldn't start.".into());
                }
            }
            let Some(get_factory) = GetProcAddress(library, PCSTR(c"GetPluginFactory".as_ptr() as *const u8)) else {
                return Err(crate::bundle::NOT_VST3.into());
            };
            let get_factory: GetPluginFactory = std::mem::transmute(get_factory);
            let factory = ComPtr::from_raw(get_factory()).ok_or("The plugin couldn't start.")?;
            if let Some(factory3) = factory.cast::<IPluginFactory3>() {
                if let Some(context) = host.as_com_ref::<IHostApplication>() {
                    factory3.setHostContext(context.as_ptr() as *mut FUnknown);
                }
            }
            let class = effect_class(&factory)?;
            Ok(Module { _library: library, factory, class })
        }
    }

    /// Makes one of the class's objects, as `I`.
    pub unsafe fn create<I: Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        let mut object: *mut c_void = std::ptr::null_mut();
        let iid = I::IID;
        let result = self.factory.createInstance(cid.as_ptr(), iid.as_ptr() as *const _, &mut object);
        if result != kResultOk {
            return None;
        }
        ComPtr::from_raw(object as *mut I)
    }
}

// The effect among the factory's classes: an audio processor that is an effect ("Fx" in its
// subcategories), else the first audio processor that isn't only an instrument.
unsafe fn effect_class(factory: &ComPtr<IPluginFactory>) -> Result<ClassInfo, String> {
    let mut vendor = String::new();
    let mut info: PFactoryInfo = std::mem::zeroed();
    if factory.getFactoryInfo(&mut info) == kResultOk {
        vendor = text(&info.vendor);
    }
    let factory2 = factory.cast::<IPluginFactory2>();
    let mut candidates: Vec<(ClassInfo, String)> = Vec::new();
    for index in 0..factory.countClasses().max(0) {
        let found = match &factory2 {
            Some(factory2) => {
                let mut info: PClassInfo2 = std::mem::zeroed();
                (factory2.getClassInfo2(index, &mut info) == kResultOk).then(|| (info.cid, text(&info.category), text(&info.name), text(&info.vendor), text(&info.subCategories)))
            }
            None => {
                let mut info: PClassInfo = std::mem::zeroed();
                (factory.getClassInfo(index, &mut info) == kResultOk).then(|| (info.cid, text(&info.category), text(&info.name), String::new(), String::new()))
            }
        };
        let Some((cid, category, name, class_vendor, subcategories)) = found else { continue };
        if category == AUDIO_CLASS {
            let vendor = if class_vendor.is_empty() { vendor.clone() } else { class_vendor };
            candidates.push((ClassInfo { cid, name, vendor }, subcategories));
        }
    }
    let has = |subcategories: &str, wanted: &str| subcategories.split('|').any(|part| part.trim().eq_ignore_ascii_case(wanted));
    if let Some((class, _)) = candidates.iter().find(|(_, subcategories)| has(subcategories, "Fx")) {
        return Ok(class.clone());
    }
    if let Some((class, _)) = candidates.iter().find(|(_, subcategories)| !has(subcategories, "Instrument")) {
        return Ok(class.clone());
    }
    Err(if candidates.is_empty() { NO_EFFECT.into() } else { INSTRUMENT.into() })
}

trait EncodeWide {
    fn encode_wide_terminated(&self) -> Vec<u16>;
}

impl EncodeWide for std::ffi::OsStr {
    fn encode_wide_terminated(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(Some(0)).collect()
    }
}
