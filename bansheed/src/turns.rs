use std::collections::HashMap;
use std::sync::Mutex;

use banshee_common::TurnVerdict;

use crate::text_to_speech::lock;

#[derive(Debug, Default, Clone, Copy)]
struct AgentTurns {
    spoke: u64,
    checked: u64,
    blocked: bool,
}

impl AgentTurns {
    fn end_turn(&mut self) -> TurnVerdict {
        if self.spoke == self.checked && !self.blocked {
            self.blocked = true;
            TurnVerdict::Speak
        } else {
            self.close_turn()
        }
    }

    fn close_turn(&mut self) -> TurnVerdict {
        self.checked = self.spoke;
        self.blocked = false;
        TurnVerdict::Pass
    }
}

/// Which agent processes spoke since their last turn ended. Kept in memory only.
#[derive(Default)]
pub struct SpokenTurns {
    agents: Mutex<HashMap<u32, AgentTurns>>,
}

impl SpokenTurns {
    /// Counts one speech or question from the agent with this process ID.
    pub fn spoke(&self, agent_pid: u32) {
        lock(&self.agents).entry(agent_pid).or_default().spoke += 1;
    }

    /// Ends the agent's turn: `Speak` when it said nothing and was not already
    /// sent back this turn.
    pub fn turn_ended(&self, agent_pid: u32) -> TurnVerdict {
        lock(&self.agents).entry(agent_pid).or_default().end_turn()
    }

    /// Closes the agent's turn on a stop that repeats one already answered: always
    /// `Pass`, and the next turn starts from here.
    pub fn repeated_stop(&self, agent_pid: u32) -> TurnVerdict {
        lock(&self.agents)
            .entry(agent_pid)
            .or_default()
            .close_turn()
    }
}

#[cfg(test)]
mod tests;
