use crate::domain::{Handoff, HandoffProgress, HandoffResponse, HandoffTermination, Task};

pub(super) fn handoff(handoff: &Handoff, task: Option<&Task>) -> String {
    let id = handoff
        .message_id()
        .expect("orchestrated handoffs have message IDs");
    let body = match (handoff.payload(), task) {
        (crate::domain::HandoffPayload::Task(_), Some(task)) => format!(
            "[OpenPodium task {id}]\nFrom agent {}\nTitle: {}\n\n{}\n\nUse `openpodium ipc progress report --handoff {id} --body <text>` for updates and `openpodium ipc respond --handoff {id} --status <completed|failed|blocked> --body <text>` when finished.",
            handoff.source(),
            sanitize(task.title().as_str()),
            sanitize(task.prompt().as_str()),
        ),
        (crate::domain::HandoffPayload::Question(body), None) => format!(
            "[OpenPodium question {id}]\nFrom agent {}\n\n{}\n\nReply with `openpodium ipc respond --handoff {id} --status completed --body <text>`.",
            handoff.source(),
            sanitize(body.as_str()),
        ),
        _ => "[OpenPodium] Invalid handoff payload".to_owned(),
    };
    sanitize(&body)
}

pub(super) fn progress(handoff: &Handoff, progress: &HandoffProgress) -> String {
    sanitize(&format!(
        "[OpenPodium progress for {}]\nFrom agent {}\n\n{}",
        handoff
            .message_id()
            .expect("orchestrated handoffs have message IDs"),
        handoff.recipient(),
        progress.body().as_str(),
    ))
}

pub(super) fn response(handoff: &Handoff, response: &HandoffResponse) -> String {
    sanitize(&format!(
        "[OpenPodium {:?} response for {}]\nFrom agent {}\n\n{}",
        response.status(),
        handoff
            .message_id()
            .expect("orchestrated handoffs have message IDs"),
        handoff.recipient(),
        response.body().as_str(),
    ))
}

pub(super) fn cancellation(handoff: &Handoff, termination: &HandoffTermination) -> String {
    let HandoffTermination::Cancelled { reason, .. } = termination else {
        return "[OpenPodium] Handoff timed out".to_owned();
    };
    sanitize(&format!(
        "[OpenPodium cancellation for {}]\nFrom agent {}\n\n{}",
        handoff
            .message_id()
            .expect("orchestrated handoffs have message IDs"),
        handoff.source(),
        reason.as_str(),
    ))
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            matches!(character, '\n' | '\t') || (!character.is_control() && *character != '\u{7f}')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::domain::{AgentId, Content, HandoffId, HandoffMessageId, HandoffPayload, Timestamp};

    use super::*;

    #[test]
    fn terminal_control_characters_are_not_emitted() {
        let tracked_handoff = Handoff::tracked(
            HandoffId::new(1),
            HandoffMessageId::new("question-1").unwrap(),
            AgentId::new(1),
            AgentId::new(2),
            HandoffPayload::Question(Content::new("safe\u{1b}[31m text").unwrap()),
            None,
            Timestamp::from_unix_millis(1),
            None,
        )
        .unwrap();

        let prompt = handoff(&tracked_handoff, None);

        assert!(!prompt.contains('\u{1b}'));
        assert!(prompt.contains("safe[31m text"));
    }
}
