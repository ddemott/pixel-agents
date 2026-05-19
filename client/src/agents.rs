// Agent state as maintained by the TUI client.
#![allow(dead_code)]

use serde::Deserialize;

/// Snapshot of a persisted agent from `agent.list` or `agent.created` events.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentSnapshot {
    pub id: i32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub palette: u8,
    #[serde(rename = "hueShift")]
    pub hue_shift: i32,
    #[serde(rename = "seatId")]
    pub seat_id: Option<String>,
}

/// Live status of an agent as observed through events.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Idle,
    Active(String), // current tool name
    Waiting,
    Exited,
}

impl AgentStatus {
    pub fn label(&self) -> &str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Active(_) => "active",
            AgentStatus::Waiting => "waiting",
            AgentStatus::Exited => "exited",
        }
    }
}

/// Combined agent state held in the client.
#[derive(Debug, Clone)]
pub struct AgentState {
    pub snapshot: AgentSnapshot,
    pub status: AgentStatus,
}

impl AgentState {
    pub fn new(snapshot: AgentSnapshot) -> Self {
        Self { snapshot, status: AgentStatus::Idle }
    }

    pub fn id(&self) -> i32 {
        self.snapshot.id
    }
}

/// Parse `agent.list` response data into a list of `AgentState`.
pub fn parse_agent_list(data: &serde_json::Value) -> Vec<AgentState> {
    data.get("agents")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value::<AgentSnapshot>(v.clone()).ok())
                .map(AgentState::new)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_empty_list() {
        let v = json!({ "agents": [] });
        assert!(parse_agent_list(&v).is_empty());
    }

    #[test]
    fn parse_single_agent() {
        let v = json!({
            "agents": [{
                "id": 1,
                "sessionId": "abc-123",
                "palette": 2,
                "hueShift": 0,
                "lastSeenAt": 1234567890
            }]
        });
        let agents = parse_agent_list(&v);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id(), 1);
        assert_eq!(agents[0].snapshot.session_id, "abc-123");
        assert_eq!(agents[0].snapshot.palette, 2);
    }

    #[test]
    fn parse_skips_malformed_entries() {
        let v = json!({
            "agents": [
                { "id": "not_a_number" },
                { "id": 2, "sessionId": "ok", "palette": 0, "hueShift": 0 }
            ]
        });
        let agents = parse_agent_list(&v);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id(), 2);
    }

    #[test]
    fn default_status_is_idle() {
        let snap = AgentSnapshot {
            id: 5,
            session_id: "x".into(),
            palette: 1,
            hue_shift: 0,
            seat_id: None,
        };
        assert_eq!(AgentState::new(snap).status, AgentStatus::Idle);
    }
}
