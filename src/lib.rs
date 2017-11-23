#[macro_use]
extern crate cstr_macro;
#[macro_use]
extern crate janus_plugin as janus;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;

mod messages;

use janus::{JanssonValue, Plugin, PluginCallbacks, PluginMetadata, PluginResult, PluginResultType,
            PluginSession, RawJanssonValue, RawPluginResult};
use janus::session::SessionWrapper;
use messages::Event;
use std::collections::HashMap;
use std::os::raw::{c_char, c_int};
use std::sync::{mpsc, Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

lazy_static! {
    static ref CHANNEL: Mutex<Option<mpsc::Sender<RawMessage>>> = Mutex::new(None);
    static ref SESSIONS: RwLock<Vec<Box<Arc<Session>>>> = RwLock::new(Vec::new());
    #[derive(Debug)]
    static ref ROOMS: RwLock<HashMap<RoomId, Box<Room>>> = RwLock::new(HashMap::new());
}

static mut GATEWAY: Option<&PluginCallbacks> = None;

#[derive(Debug)]
struct RawMessage {
    session: Weak<Session>,
    transaction: *mut c_char,
    message: Option<JanssonValue>,
    jsep: Option<JanssonValue>,
}
unsafe impl std::marker::Send for RawMessage {}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct RoomId(u64);

#[derive(Debug)]
struct Room {
    id: RoomId,
    caller: Option<Weak<Session>>,
    callee: Option<Weak<Session>>,
}

impl Room {
    fn new(id: RoomId) -> Room {
        Room {
            id,
            callee: None,
            caller: None,
        }
    }

    fn is_new(id: RoomId) -> bool {
        let rooms = ROOMS.read().unwrap();
        !rooms.contains_key(&id)
    }

    fn is_empty(&self) -> bool {
        self.caller.is_none() && self.callee.is_none()
    }

    fn create(this: Room) {
        let mut rooms = ROOMS.write().unwrap();
        rooms.insert(this.id, Box::new(this));
    }

    fn get_mut(rooms: &mut HashMap<RoomId, Box<Room>>, id: RoomId) -> &mut Box<Room> {
        rooms.get_mut(&id).unwrap()
    }

    fn all() -> RwLockReadGuard<'static, HashMap<RoomId, Box<Room>>> {
        ROOMS.read().expect("Cannot lock ROOMS for read")
    }

    fn all_mut() -> RwLockWriteGuard<'static, HashMap<RoomId, Box<Room>>> {
        ROOMS.write().expect("Cannot lock ROOMS for write")
    }

    fn add_member(&mut self, member: RoomMember) {
        match member {
            RoomMember::Callee(ref session) => {
                self.callee = Some(Arc::downgrade(session));
            }
            RoomMember::Caller(ref session) => {
                self.caller = Some(Arc::downgrade(session));
            }
        }
    }
}

#[derive(Debug)]
enum RoomMember {
    Callee(Arc<Session>),
    Caller(Arc<Session>),
}

#[derive(Debug)]
struct SessionState {
    room_id: Option<RoomId>,
    initiator: Option<bool>,
}

impl SessionState {
    fn get(session: &Session) -> RwLockReadGuard<SessionState> {
        // deref Arc, deref SessionWrapper
        session.read().expect("Cannot lock session for read")
    }

    fn get_mut(session: &Session) -> RwLockWriteGuard<SessionState> {
        session.write().expect("Cannot lock session for write")
    }

    fn get_room<'a>(&self, rooms: &'a HashMap<RoomId, Box<Room>>) -> &'a Box<Room> {
        rooms.get(&self.room_id.expect("Session state has no room id")).unwrap()
    }
}

type Session = SessionWrapper<RwLock<SessionState>>;

extern "C" fn init(callback: *mut PluginCallbacks, _config_path: *const c_char) -> c_int {
    janus::log(janus::LogLevel::Verb, "--> P2P init");

    unsafe {
        let callback = callback.as_ref().unwrap();
        GATEWAY = Some(callback);
    }

    let (tx, rx) = mpsc::channel();
    *(CHANNEL.lock().unwrap()) = Some(tx);

    std::thread::spawn(move || {
        janus::log(janus::LogLevel::Verb, "--> P2P Start handling thread");

        for msg in rx.iter() {
            janus::log(
                janus::LogLevel::Verb,
                &format!("Processing message: {:?}", msg),
            );
            handle_message_async(msg);
        }
    });

    0
}

extern "C" fn destroy() {
    janus::log(janus::LogLevel::Verb, "--> P2P destroy");
}

extern "C" fn create_session(handle: *mut PluginSession, error: *mut c_int) {
    let state = SessionState {
        room_id: None,
        initiator: None,
    };
    match Session::associate(handle, RwLock::new(state)) {
        Ok(session) => {
            janus::log(
                janus::LogLevel::Info,
                &format!("Initializing P2P session {:?}", session),
            );
            SESSIONS.write().unwrap().push(session);
        }
        Err(e) => {
            janus::log(janus::LogLevel::Err, &format!("{}", e));
            unsafe { *error = -1 };
        }
    }
}

extern "C" fn query_session(_handle: *mut PluginSession) -> *mut RawJanssonValue {
    janus::log(janus::LogLevel::Verb, "--> P2P query_session");
    std::ptr::null_mut()
}

extern "C" fn destroy_session(handle: *mut PluginSession, _error: *mut c_int) {
    janus::log(janus::LogLevel::Verb, "--> P2P destroy_session");

    let session = Session::from_ptr(handle).unwrap();
    let state = SessionState::get(&session);

    let mut rooms = Room::all_mut();
    let room_id = state.room_id.unwrap();

    let is_empty = {
        let room = Room::get_mut(&mut rooms, room_id);

        SESSIONS.write().unwrap().retain(|ref s| s.as_ptr() != handle);

        match state.initiator {
            Some(true) => room.caller = None,
            Some(false) | None => room.callee = None,
        }

        room.is_empty()
    };

    if is_empty {
        janus::log(
            janus::LogLevel::Verb,
            &format!("Room #{:?} is empty, removing it.", room_id),
        );

        rooms.remove(&room_id).expect("Room must be present in HashMap");
    } else {
        janus::log(
            janus::LogLevel::Verb,
            &format!("Room #{:?} is not empty yet.", room_id),
        );
    }
}

extern "C" fn handle_message(
    handle: *mut PluginSession,
    transaction: *mut c_char,
    message: *mut RawJanssonValue,
    jsep: *mut RawJanssonValue,
) -> *mut RawPluginResult {
    janus::log(janus::LogLevel::Verb, "--> P2P handle_message");

    let result = match Session::from_ptr(handle) {
        Ok(ref session) => {
            let message = RawMessage {
                session: Arc::downgrade(session),
                transaction: transaction,
                message: unsafe { JanssonValue::new(message) },
                jsep: unsafe { JanssonValue::new(jsep) },
            };

            let mutex = CHANNEL.lock().unwrap();
            let tx = mutex.as_ref().unwrap();

            janus::log(janus::LogLevel::Verb, "--> P2P sending message to channel");
            tx.send(message).expect("Sending to channel has failed");

            PluginResult::new(
                PluginResultType::JANUS_PLUGIN_OK_WAIT,
                std::ptr::null(),
                None,
            )
        }
        Err(_) => PluginResult::new(
            PluginResultType::JANUS_PLUGIN_ERROR,
            cstr!("No handle associated with session"),
            None,
        ),
    };
    result.into_raw()
}

extern "C" fn setup_media(_handle: *mut PluginSession) {
    janus::log(janus::LogLevel::Verb, "--> P2P setup_media");
}

extern "C" fn hangup_media(_handle: *mut PluginSession) {
    janus::log(janus::LogLevel::Verb, "--> P2P hangup_media");
}

extern "C" fn incoming_rtp(
    _handle: *mut PluginSession,
    _video: c_int,
    _buf: *mut c_char,
    _len: c_int,
) {
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

fn handle_message_async(msg: RawMessage) {
    let RawMessage {
        session,
        transaction,
        message,
        ..
    } = msg;

    let message: JanssonValue = message.unwrap();
    let event = messages::parse(message);
    println!("\n--> got message: {:?}", event);

    if let Some(session) = session.upgrade() {
        match event {
            Event::Join { room_id, initiator } => {
                let mut state = SessionState::get_mut(&session);

                match state.room_id {
                    Some(x) => println!("Session already has room_id set: {:?}", x),
                    None => state.room_id = Some(room_id),
                }
                match state.initiator {
                    Some(x) => println!("Session already has initiator set: {:?}", x),
                    None => state.initiator = Some(initiator),
                }

                // {
                //     let rooms = ROOMS.read().unwrap();
                //     println!("--> rooms before: {:?}", *rooms);
                // }

                let is_new_room = Room::is_new(room_id);

                if initiator {
                    if is_new_room {
                        let mut room = Room::new(room_id);
                        println!("--> caller is joining new room: #{:?}", room);

                        room.add_member(RoomMember::Caller(session.clone()));
                        println!("--> room after adding caller: {:?}", room);

                        Room::create(room);
                    } else {
                        let mut rooms = Room::all_mut();
                        let room = Room::get_mut(&mut rooms, room_id);

                        println!("--> caller is joining existing room: {:?}", room);

                        room.add_member(RoomMember::Caller(session.clone()));
                        println!("--> room after adding caller: {:?}", room);
                    }
                } else {
                    if is_new_room {
                        let mut room = Room::new(room_id);
                        println!("--> callee is joining new room: #{:?}", room);

                        room.add_member(RoomMember::Callee(session.clone()));
                        println!("--> room after adding callee: {:?}", room);

                        Room::create(room);
                    } else {
                        let mut rooms = Room::all_mut();
                        let room = Room::get_mut(&mut rooms, room_id);

                        println!("--> callee is joining existing room: #{:?}", room);

                        room.add_member(RoomMember::Callee(session.clone()));
                        println!("--> room after adding callee: {:?}", room);
                    }
                }
            }
            jsep @ Event::Call { .. } => {
                println!("--> handle call event");

                let state = SessionState::get(&session);
                println!("--> state: {:?}", state);

                let rooms = Room::all();
                let room = state.get_room(&rooms);
                println!("--> room: {:?}", room);

                if let Some(ref caller) = room.callee {
                    let peer = caller.upgrade().expect("Caller has gone");
                    println!("--> callee: {:?}", peer);
                    println!("--> pushing offer to callee");
                    relay_jsep(&peer, transaction, jsep);
                }
            }
            jsep @ Event::Accept { .. } => {
                println!("--> handle accept event");

                let state = SessionState::get(&session);
                println!("--> state: {:?}", state);

                let rooms = Room::all();
                let room = state.get_room(&rooms);
                println!("--> room: {:?}", room);

                if let Some(ref caller) = room.caller {
                    let peer = caller.upgrade().expect("Callee has gone");
                    println!("--> caller: {:?}", peer);
                    println!("--> pushing answer to caller");
                    relay_jsep(&peer, transaction, jsep);
                }
            }
            candidate @ Event::Candidate { .. } => {
                println!("--> handle candidate event");

                let state = SessionState::get(&session);
                println!("--> state: {:?}", state);

                let rooms = Room::all();
                let room = state.get_room(&rooms);
                println!("--> room: {:?}", room);

                match state.initiator {
                    Some(true) => if let Some(ref x) = room.callee {
                        let peer = x.upgrade().unwrap();
                        println!("-->> pushing ICE to callee");
                        relay_ice_candidate(&peer, transaction, candidate);
                    },
                    Some(false) | None => if let Some(ref x) = room.caller {
                        let peer = x.upgrade().unwrap();
                        println!("-->> pushing ICE to caller");
                        relay_ice_candidate(&peer, transaction, candidate);
                    },
                }
            }
        }
    } else {
        janus::log(
            janus::LogLevel::Warn,
            "Got a message for destroyed session.",
        );
    }
}

fn relay_jsep(peer: &Session, transaction: *mut c_char, jsep: Event) {
    let json = prepare_response(&jsep);
    let event = serde_into_jansson(json);

    let push_event_fn = acquire_gateway().push_event;
    janus::get_result(push_event_fn(
        peer.handle,
        &mut PLUGIN,
        transaction,
        event.as_mut_ref(),
        std::ptr::null_mut(),
    )).expect("Pushing jsep has failed");
}

fn relay_ice_candidate(peer: &Session, transaction: *mut c_char, candidate: Event) {
    let json = prepare_response(&candidate);
    let event = serde_into_jansson(json);

    let push_event_fn = acquire_gateway().push_event;
    janus::get_result(push_event_fn(
        peer.handle,
        &mut PLUGIN,
        transaction,
        event.as_mut_ref(),
        std::ptr::null_mut(),
    )).expect("Pushing ice has failed");
}

fn prepare_response(event: &Event) -> serde_json::Value {
    let mut event_json = json!(event);
    {
        let json_obj = event_json.as_object_mut().unwrap();
        json_obj.entry("result").or_insert(json!("ok"));
    }
    event_json
}

fn serde_into_jansson(value: serde_json::Value) -> JanssonValue {
    JanssonValue::from_str(&value.to_string(), janus::JanssonDecodingFlags::empty()).unwrap()
}

fn acquire_gateway() -> &'static PluginCallbacks {
    unsafe { GATEWAY }.expect("Gateway is NONE")
}

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
