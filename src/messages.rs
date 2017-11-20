use super::{serde_json, Room, RoomId, RoomMember, Session, SessionState};
use janus::{JanssonEncodingFlags, JanssonValue};
use std::error;
use std::fmt;
use std::sync::{Arc, Weak};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "event")]
enum Event {
    Join { room_id: RoomId, initiator: bool },
    Call { jsep: JsepKind },
    Accept { jsep: JsepKind },
    Candidate { candidate: IceCandidate },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
enum JsepKind {
    Offer { sdp: String },
    Answer { sdp: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IceCandidate {
    candidate: String,
    sdp_mid: String,
    sdp_m_line_index: u16,
}

#[derive(Debug)]
pub enum Response {
    Join {
        peer: Weak<Session>,
        payload: serde_json::Value,
    },
    Call {
        peer: Weak<Session>,
        payload: serde_json::Value,
    },
    Accept {
        peer: Weak<Session>,
        payload: serde_json::Value,
    },
    Candidate {
        peer: Weak<Session>,
        payload: serde_json::Value,
    },
}

#[derive(Debug)]
pub enum Error {
    EmptyPeer,
    PeerHasGone,
}

impl error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::EmptyPeer => "Peer is empty",
            Error::PeerHasGone => "Peer has gone",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::EmptyPeer => write!(f, "Peer is empty"),
            Error::PeerHasGone => write!(f, "Peer has gone"),
        }
    }
}

pub fn process(
    session: &Arc<Session>,
    msg: JanssonValue,
) -> ::std::result::Result<Response, Box<error::Error>> {
    let value_string: String = msg.to_string(JanssonEncodingFlags::empty());
    let event: Event = serde_json::from_str(&value_string)?;
    println!("\n--> got message: {:?}", event);

    match event {
        // TODO: respond wheither or not a room is ready for conversation
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
                let caller = session.clone();

                if is_new_room {
                    let mut room = Room::new(room_id);
                    println!("--> caller is joining new room: #{:?}", room);

                    room.add_member(RoomMember::Caller(caller));
                    println!("--> room after adding caller: {:?}", room);

                    Room::create(room);
                } else {
                    let mut rooms = Room::all_mut();
                    let room = Room::get_mut(&mut rooms, room_id);

                    println!("--> caller is joining existing room: {:?}", room);

                    room.add_member(RoomMember::Caller(caller));
                    println!("--> room after adding caller: {:?}", room);
                }

                let resp = Response::Join {
                    peer: Arc::downgrade(session),
                    payload: json!({ "result": "join", "full": false, "you": "caller" }),
                };
                Ok(resp)
            } else {
                let callee = session.clone();

                if is_new_room {
                    let mut room = Room::new(room_id);
                    println!("--> callee is joining new room: #{:?}", room);

                    room.add_member(RoomMember::Callee(callee));
                    println!("--> room after adding callee: {:?}", room);

                    Room::create(room);
                } else {
                    let mut rooms = Room::all_mut();
                    let room = Room::get_mut(&mut rooms, room_id);

                    println!("--> callee is joining existing room: #{:?}", room);

                    room.add_member(RoomMember::Callee(callee));
                    println!("--> room after adding callee: {:?}", room);
                }

                let resp = Response::Join {
                    peer: Arc::downgrade(session),
                    payload: json!({ "result": "join", "full": false, "you": "callee" }),
                };
                Ok(resp)
            }
        }
        jsep @ Event::Call { .. } => {
            println!("--> handle call event");

            let state = SessionState::get(&session);
            println!("--> state: {:?}", state);

            let rooms = Room::all();
            let room = state.get_room(&rooms);
            println!("--> room: {:?}", room);

            if let Some(ref callee) = room.callee {
                let resp = Response::Call {
                    peer: callee.clone(),
                    payload: json!(jsep),
                };
                Ok(resp)
            } else {
                Err(Error::EmptyPeer)?
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
                let resp = Response::Accept {
                    peer: caller.clone(),
                    payload: json!(jsep),
                };
                Ok(resp)
            } else {
                Err(Error::EmptyPeer)?
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
                    let resp = Response::Candidate {
                        peer: x.clone(),
                        payload: json!(candidate),
                    };
                    Ok(resp)
                } else {
                    Err(Error::EmptyPeer)?
                },
                Some(false) | None => if let Some(ref x) = room.caller {
                    let resp = Response::Candidate {
                        peer: x.clone(),
                        payload: json!(candidate),
                    };
                    Ok(resp)
                } else {
                    Err(Error::EmptyPeer)?
                },
            }
        }
    }
}
