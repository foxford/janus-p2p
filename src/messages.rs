use super::{serde_json, RoomId};
use janus::{JanssonEncodingFlags, JanssonValue};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "event")]
pub enum Event {
    Join { room_id: RoomId, initiator: bool },
    Call { jsep: JsepKind },
    Accept { jsep: JsepKind },
    Candidate { candidate: IceCandidate },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum JsepKind {
    Offer { sdp: String },
    Answer { sdp: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceCandidate {
    candidate: String,
    sdp_mid: String,
    sdp_m_line_index: u16,
}

pub fn parse(msg: JanssonValue) -> Event {
    let value_string: String = msg.to_string(JanssonEncodingFlags::empty());
    serde_json::from_str(&value_string).unwrap()
}
