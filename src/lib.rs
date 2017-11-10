#[macro_use]
extern crate cstr_macro;
#[macro_use]
extern crate janus_plugin as janus;
#[macro_use]
extern crate lazy_static;

use std::os::raw::{c_char, c_int, c_void};
use std::sync::{mpsc, Mutex, RwLock};
use janus::{JanssonValue, Plugin, PluginCallbacks, PluginMetadata, PluginResult, PluginResultType,
            PluginSession, RawJanssonValue, RawPluginResult};

lazy_static! {
    static ref CHANNEL: Mutex<Option<mpsc::Sender<Message>>> = Mutex::new(None);
    static ref SESSIONS: RwLock<Vec<Box<Session>>> = RwLock::new(Vec::new());
}

static mut GATEWAY: Option<&PluginCallbacks> = None;

#[derive(Debug)]
struct Message {
    handle: *mut PluginSession,
    transaction: *mut c_char,
    message: Option<JanssonValue>,
    jsep: Option<JanssonValue>,
}
unsafe impl std::marker::Send for Message {}

#[derive(Debug)]
struct Session {}

extern "C" fn init(callback: *mut PluginCallbacks, _config_path: *const c_char) -> c_int {
    janus::log(janus::LogLevel::Verb, "--> P2P init");

    unsafe {
        let callback = callback.as_ref().unwrap();
        GATEWAY = Some(callback);
    }

    let (tx, rx) = mpsc::channel();
    *(CHANNEL.lock().unwrap()) = Some(tx);

    std::thread::spawn(move || {
        message_handler(rx);
    });

    0
}

extern "C" fn destroy() {
    janus::log(janus::LogLevel::Verb, "--> P2P destroy");
}

extern "C" fn create_session(handle: *mut PluginSession, _error: *mut c_int) {
    janus::log(janus::LogLevel::Verb, "--> P2P create_session");

    let handle = unsafe { &mut *handle };
    let mut session = Box::new(Session {});

    handle.plugin_handle = session.as_mut() as *mut Session as *mut c_void;
    SESSIONS.write().unwrap().push(session);
}

extern "C" fn query_session(_handle: *mut PluginSession) -> *mut RawJanssonValue {
    janus::log(janus::LogLevel::Verb, "--> P2P query_session");
    std::ptr::null_mut()
}

extern "C" fn destroy_session(_handle: *mut PluginSession, _error: *mut c_int) {
    janus::log(janus::LogLevel::Verb, "--> P2P destroy_session");
}

extern "C" fn handle_message(
    handle: *mut PluginSession,
    transaction: *mut c_char,
    message: *mut RawJanssonValue,
    jsep: *mut RawJanssonValue,
) -> *mut RawPluginResult {
    janus::log(janus::LogLevel::Verb, "--> P2P handle_message");

    janus::log(janus::LogLevel::Verb, "--> P2P acquiring transfer lock");
    let mutex = CHANNEL.lock().unwrap();
    let tx = mutex.as_ref().unwrap();
    janus::log(janus::LogLevel::Verb, "--> P2P acquired transfer lock");

    let message = unsafe { JanssonValue::new(message) };
    let jsep = unsafe { JanssonValue::new(jsep) };

    let echo_message = Message {
        handle: handle,
        transaction: transaction,
        message: message,
        jsep: jsep,
    };
    janus::log(janus::LogLevel::Verb, "--> P2P sending message to channel");
    tx.send(echo_message)
        .expect("Sending to channel has failed");

    let result = PluginResult::new(
        PluginResultType::JANUS_PLUGIN_OK_WAIT,
        std::ptr::null(),
        None,
    );
    result.into_raw()
}

extern "C" fn setup_media(_handle: *mut PluginSession) {
    janus::log(janus::LogLevel::Verb, "--> P2P setup_media");
}

extern "C" fn hangup_media(_handle: *mut PluginSession) {
    janus::log(janus::LogLevel::Verb, "--> P2P hangup_media");
}

extern "C" fn incoming_rtp(handle: *mut PluginSession, video: c_int, buf: *mut c_char, len: c_int) {
    // let relay_fn = acquire_gateway().relay_rtp;
    // relay_fn(handle, video, buf, len);
}

extern "C" fn incoming_rtcp(
    _handle: *mut PluginSession,
    _video: c_int,
    _buf: *mut c_char,
    _len: c_int,
) {
}

extern "C" fn incoming_data(_handle: *mut PluginSession, _buf: *mut c_char, _len: c_int) {}

extern "C" fn slow_link(_handle: *mut PluginSession, _uplink: c_int, _video: c_int) {}

fn message_handler(rx: mpsc::Receiver<Message>) {}

const METADATA: PluginMetadata = PluginMetadata {
    version: 1,
    version_str: cstr!("0.1"),
    description: cstr!("P2P plugin"),
    name: cstr!("P2P plugin"),
    author: cstr!("Aleksey Ivanov"),
    package: cstr!("janus.plugin.p2p"),
};

const PLUGIN: Plugin = build_plugin!(
    METADATA,
    init,
    destroy,
    create_session,
    handle_message,
    setup_media,
    incoming_rtp,
    incoming_rtcp,
    incoming_data,
    slow_link,
    hangup_media,
    destroy_session,
    query_session
);

export_plugin!(&PLUGIN);
