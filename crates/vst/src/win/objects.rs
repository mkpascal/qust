// The objects a VST3 host hands its plugins: the host application (which also makes messages and
// attribute lists, for plugins whose processor and controller talk to each other), memory
// streams for their states, the component handler (the controller reporting edits), the
// parameter queues of each processing block, and the frame of an editor window.
//
// Plugins may call any of them from any thread, so none touches the host's state: they keep
// what they are told (in atomics, rings and mutexes) for the plugin thread to pick up.

use crate::ring::ParamRing;
use std::cell::{Cell, UnsafeCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComRef, ComWrapper, Interface};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) unsafe fn guid(tuid: *const TUID) -> Option<[u8; 16]> {
    (!tuid.is_null()).then(|| *(tuid as *const [u8; 16]))
}

/// A 128-character VST string from `text` (cut, always terminated).
pub(crate) unsafe fn write_string128(target: *mut String128, text: &str) {
    if target.is_null() {
        return;
    }
    let target = &mut *target;
    let mut length = 0;
    for (slot, unit) in target.iter_mut().zip(text.encode_utf16().take(127)) {
        *slot = unit;
        length += 1;
    }
    target[length] = 0;
}

/// A VST `char8` array (UTF-8, terminated) as text.
pub(crate) fn text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/* The host application */

pub struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication, IPlugInterfaceSupport);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        write_string128(name, "Qust");
        kResultOk
    }

    unsafe fn createInstance(&self, cid: *mut TUID, iid: *mut TUID, obj: *mut *mut c_void) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        *obj = std::ptr::null_mut();
        let (Some(cid), Some(iid)) = (guid(cid), guid(iid)) else { return kInvalidArgument };
        let made = if cid == IMessage::IID && iid == IMessage::IID {
            ComWrapper::new(Message::default()).to_com_ptr::<IMessage>().map(|ptr| ptr.into_raw() as *mut c_void)
        } else if cid == IAttributeList::IID && iid == IAttributeList::IID {
            ComWrapper::new(Attributes::default()).to_com_ptr::<IAttributeList>().map(|ptr| ptr.into_raw() as *mut c_void)
        } else {
            None
        };
        match made {
            Some(ptr) => {
                *obj = ptr;
                kResultOk
            }
            None => kNoInterface,
        }
    }
}

impl IPlugInterfaceSupportTrait for HostApp {
    unsafe fn isPlugInterfaceSupported(&self, iid: *const TUID) -> tresult {
        let Some(iid) = guid(iid) else { return kInvalidArgument };
        let supported = [
            IComponent::IID,
            IAudioProcessor::IID,
            IEditController::IID,
            IConnectionPoint::IID,
            IPlugView::IID,
            IPlugFrame::IID,
            IPlugViewContentScaleSupport::IID,
            IComponentHandler::IID,
            IHostApplication::IID,
            IMessage::IID,
            IAttributeList::IID,
            IBStream::IID,
            ISizeableStream::IID,
            IParameterChanges::IID,
            IParamValueQueue::IID,
        ];
        if supported.contains(&iid) {
            kResultTrue
        } else {
            kResultFalse
        }
    }
}

/* Messages and attribute lists */

pub struct Message {
    id: Mutex<Option<CString>>,
    attributes: ComWrapper<Attributes>,
}

impl Default for Message {
    fn default() -> Self {
        Message { id: Mutex::new(None), attributes: ComWrapper::new(Attributes::default()) }
    }
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        // The string lives as long as the message, or until the id is set again.
        lock(&self.id).as_ref().map_or(std::ptr::null(), |id| id.as_ptr())
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        *lock(&self.id) = (!id.is_null()).then(|| CStr::from_ptr(id).to_owned());
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Not counted: the message keeps it (as the SDK's host message does).
        self.attributes.as_com_ref::<IAttributeList>().map_or(std::ptr::null_mut(), |list| list.as_ptr())
    }
}

enum Attribute {
    Int(i64),
    Float(f64),
    // UTF-16, terminated.
    String(Vec<u16>),
    Binary(Vec<u8>),
}

#[derive(Default)]
pub struct Attributes {
    values: Mutex<HashMap<CString, Attribute>>,
}

impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}

unsafe fn attribute_id(id: *const c_char) -> Option<CString> {
    (!id.is_null()).then(|| CStr::from_ptr(id).to_owned())
}

impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: *const c_char, value: int64) -> tresult {
        let Some(id) = attribute_id(id) else { return kInvalidArgument };
        lock(&self.values).insert(id, Attribute::Int(value));
        kResultOk
    }

    unsafe fn getInt(&self, id: *const c_char, value: *mut int64) -> tresult {
        match (attribute_id(id), value.is_null()) {
            (Some(id), false) => match lock(&self.values).get(&id) {
                Some(Attribute::Int(stored)) => {
                    *value = *stored;
                    kResultOk
                }
                _ => kResultFalse,
            },
            _ => kInvalidArgument,
        }
    }

    unsafe fn setFloat(&self, id: *const c_char, value: f64) -> tresult {
        let Some(id) = attribute_id(id) else { return kInvalidArgument };
        lock(&self.values).insert(id, Attribute::Float(value));
        kResultOk
    }

    unsafe fn getFloat(&self, id: *const c_char, value: *mut f64) -> tresult {
        match (attribute_id(id), value.is_null()) {
            (Some(id), false) => match lock(&self.values).get(&id) {
                Some(Attribute::Float(stored)) => {
                    *value = *stored;
                    kResultOk
                }
                _ => kResultFalse,
            },
            _ => kInvalidArgument,
        }
    }

    unsafe fn setString(&self, id: *const c_char, string: *const TChar) -> tresult {
        let Some(id) = attribute_id(id) else { return kInvalidArgument };
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut units = Vec::new();
        let mut at = string;
        while *at != 0 && units.len() < 1 << 20 {
            units.push(*at);
            at = at.add(1);
        }
        units.push(0);
        lock(&self.values).insert(id, Attribute::String(units));
        kResultOk
    }

    unsafe fn getString(&self, id: *const c_char, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        let Some(id) = attribute_id(id) else { return kInvalidArgument };
        let capacity = size_in_bytes as usize / 2;
        if string.is_null() || capacity == 0 {
            return kInvalidArgument;
        }
        match lock(&self.values).get(&id) {
            Some(Attribute::String(units)) => {
                let count = units.len().min(capacity);
                std::ptr::copy_nonoverlapping(units.as_ptr(), string, count);
                *string.add(count - 1) = 0;
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setBinary(&self, id: *const c_char, data: *const c_void, size_in_bytes: uint32) -> tresult {
        let Some(id) = attribute_id(id) else { return kInvalidArgument };
        let bytes = if data.is_null() || size_in_bytes == 0 { Vec::new() } else { std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize).to_vec() };
        lock(&self.values).insert(id, Attribute::Binary(bytes));
        kResultOk
    }

    unsafe fn getBinary(&self, id: *const c_char, data: *mut *const c_void, size_in_bytes: *mut uint32) -> tresult {
        let Some(id) = attribute_id(id) else { return kInvalidArgument };
        if data.is_null() || size_in_bytes.is_null() {
            return kInvalidArgument;
        }
        match lock(&self.values).get(&id) {
            // Valid while the list lives and the value isn't replaced, as in the SDK.
            Some(Attribute::Binary(bytes)) => {
                *data = bytes.as_ptr() as *const c_void;
                *size_in_bytes = bytes.len() as uint32;
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

/* Memory streams: a plugin's state, written by getState and read by setState */

#[derive(Default)]
pub struct Stream {
    // the bytes and the position
    data: Mutex<(Vec<u8>, usize)>,
}

impl Stream {
    pub fn with(bytes: Vec<u8>) -> Stream {
        Stream { data: Mutex::new((bytes, 0)) }
    }

    pub fn bytes(&self) -> Vec<u8> {
        lock(&self.data).0.clone()
    }

    pub fn rewind(&self) {
        lock(&self.data).1 = 0;
    }
}

impl Class for Stream {
    type Interfaces = (IBStream, ISizeableStream);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
        if buffer.is_null() || count < 0 {
            return kInvalidArgument;
        }
        let mut data = lock(&self.data);
        let (bytes, position) = &mut *data;
        let available = bytes.len().saturating_sub(*position).min(count as usize);
        std::ptr::copy_nonoverlapping(bytes.as_ptr().add(*position), buffer as *mut u8, available);
        *position += available;
        if !read.is_null() {
            *read = available as int32;
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, count: int32, written: *mut int32) -> tresult {
        if buffer.is_null() || count < 0 {
            return kInvalidArgument;
        }
        let mut data = lock(&self.data);
        let (bytes, position) = &mut *data;
        let end = *position + count as usize;
        if bytes.len() < end {
            bytes.resize(end, 0);
        }
        std::ptr::copy_nonoverlapping(buffer as *const u8, bytes.as_mut_ptr().add(*position), count as usize);
        *position = end;
        if !written.is_null() {
            *written = count;
        }
        kResultOk
    }

    unsafe fn seek(&self, offset: int64, mode: int32, result: *mut int64) -> tresult {
        let mut data = lock(&self.data);
        let (bytes, position) = &mut *data;
        let base = match mode {
            IBStream_::IStreamSeekMode_::kIBSeekSet => 0,
            IBStream_::IStreamSeekMode_::kIBSeekCur => *position as i64,
            IBStream_::IStreamSeekMode_::kIBSeekEnd => bytes.len() as i64,
            _ => return kInvalidArgument,
        };
        let Some(target) = base.checked_add(offset).filter(|target| *target >= 0) else { return kInvalidArgument };
        *position = target as usize;
        if !result.is_null() {
            *result = target;
        }
        kResultOk
    }

    unsafe fn tell(&self, position: *mut int64) -> tresult {
        if position.is_null() {
            return kInvalidArgument;
        }
        *position = lock(&self.data).1 as int64;
        kResultOk
    }
}

impl ISizeableStreamTrait for Stream {
    unsafe fn getStreamSize(&self, size: *mut int64) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        *size = lock(&self.data).0.len() as int64;
        kResultOk
    }

    unsafe fn setStreamSize(&self, size: int64) -> tresult {
        if size < 0 {
            return kInvalidArgument;
        }
        lock(&self.data).0.resize(size as usize, 0);
        kResultOk
    }
}

/* The component handler: the controller's edits, for the processor and for saving */

pub struct Handler {
    pub to_audio: Arc<ParamRing>,
    // The ring takes one producer at a time; plugins may report edits from more than one thread.
    producer: Mutex<()>,
    /// The settings changed since the state was last saved.
    pub changed: AtomicBool,
    /// `restartComponent` flags not yet acted on.
    pub restart: AtomicI32,
}

impl Handler {
    pub fn new(to_audio: Arc<ParamRing>) -> Handler {
        Handler { to_audio, producer: Mutex::new(()), changed: AtomicBool::new(false), restart: AtomicI32::new(0) }
    }
}

impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        let _one = lock(&self.producer);
        self.to_audio.push(id, value);
        self.changed.store(true, Ordering::Release);
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        self.changed.store(true, Ordering::Release);
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.restart.fetch_or(flags, Ordering::AcqRel);
        self.changed.store(true, Ordering::Release);
        kResultOk
    }
}

/* One block's parameter changes, on the output thread: fixed sizes, nothing allocated */

const POINTS: usize = 16;
pub const QUEUES: usize = 64;

pub struct Queue {
    id: Cell<ParamID>,
    points: UnsafeCell<[(int32, ParamValue); POINTS]>,
    count: Cell<usize>,
}

impl Class for Queue {
    type Interfaces = (IParamValueQueue,);
}

impl Queue {
    fn new() -> Queue {
        Queue { id: Cell::new(0), points: UnsafeCell::new([(0, 0.0); POINTS]), count: Cell::new(0) }
    }

    pub fn id(&self) -> ParamID {
        self.id.get()
    }

    /// The last value the block carried.
    pub fn last(&self) -> Option<ParamValue> {
        let count = self.count.get();
        (count > 0).then(|| unsafe { (*self.points.get())[count - 1].1 })
    }
}

impl IParamValueQueueTrait for Queue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.count.get() as int32
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        if index < 0 || index as usize >= self.count.get() || offset.is_null() || value.is_null() {
            return kResultFalse;
        }
        let (at, stored) = (*self.points.get())[index as usize];
        *offset = at;
        *value = stored;
        kResultOk
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let points = &mut *self.points.get();
        let count = self.count.get();
        // Full: the last point takes the newest value.
        let at = if count < POINTS {
            self.count.set(count + 1);
            count
        } else {
            POINTS - 1
        };
        points[at] = (offset, value);
        if !index.is_null() {
            *index = at as int32;
        }
        kResultOk
    }
}

pub struct Changes {
    queues: Vec<ComWrapper<Queue>>,
    count: Cell<usize>,
}

impl Class for Changes {
    type Interfaces = (IParameterChanges,);
}

impl Changes {
    pub fn new() -> Changes {
        Changes { queues: (0..QUEUES).map(|_| ComWrapper::new(Queue::new())).collect(), count: Cell::new(0) }
    }

    pub fn clear(&self) {
        self.count.set(0);
    }

    fn queue(&self, id: ParamID) -> Option<&ComWrapper<Queue>> {
        let count = self.count.get();
        if let Some(index) = self.queues[..count].iter().position(|queue| queue.id.get() == id) {
            return Some(&self.queues[index]);
        }
        let queue = self.queues.get(count)?;
        queue.id.set(id);
        queue.count.set(0);
        self.count.set(count + 1);
        Some(queue)
    }

    /// A change from the editor, at the start of the block (the newest one wins).
    pub fn set(&self, id: ParamID, value: ParamValue) {
        if let Some(queue) = self.queue(id) {
            unsafe {
                (*queue.points.get())[0] = (0, value);
            }
            queue.count.set(1);
        }
    }

    /// The queues the plugin filled.
    pub fn filled(&self) -> &[ComWrapper<Queue>] {
        &self.queues[..self.count.get()]
    }
}

impl IParameterChangesTrait for Changes {
    unsafe fn getParameterCount(&self) -> int32 {
        self.count.get() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.count.get() {
            return std::ptr::null_mut();
        }
        self.queues[index as usize].as_com_ref::<IParamValueQueue>().map_or(std::ptr::null_mut(), |queue| queue.as_ptr())
    }

    unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        let Some(queue) = self.queue(*id) else { return std::ptr::null_mut() };
        if !index.is_null() {
            *index = self.filled().iter().position(|q| q.id.get() == *id).unwrap_or(0) as int32;
        }
        queue.as_com_ref::<IParamValueQueue>().map_or(std::ptr::null_mut(), |queue| queue.as_ptr())
    }
}

/* An editor window's frame: the plugin asks for a new size */

pub struct Frame {
    pub hwnd: AtomicIsize,
    /// Set while the window follows the plugin's size, so that resize isn't echoed back.
    pub resizing: AtomicBool,
}

impl Class for Frame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, view: *mut IPlugView, size: *mut ViewRect) -> tresult {
        let (Some(view), false) = (ComRef::from_raw(view), size.is_null()) else { return kInvalidArgument };
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd == 0 {
            return kResultFalse;
        }
        let rect = *size;
        self.resizing.store(true, Ordering::Release);
        super::editor::set_client_size(hwnd, rect.right - rect.left, rect.bottom - rect.top);
        self.resizing.store(false, Ordering::Release);
        let mut rect = rect;
        view.onSize(&mut rect);
        kResultOk
    }
}
