use iced::Element;
use iced::widget::{button, column, row, text};
use openpodium::supervisor::{NotificationSettings, SignalClass, Snapshot};

#[derive(Debug, Clone, Copy)]
pub enum Message {
    ToggleNotifications(SignalClass),
    CycleCooldown(SignalClass),
}

#[derive(Default)]
pub struct UiState {
    notifications: NotificationSettings,
}

impl UiState {
    pub fn notification_settings(&self) -> &NotificationSettings {
        &self.notifications
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ToggleNotifications(class) => self.notifications.toggle(class),
            Message::CycleCooldown(class) => self.notifications.cycle_cooldown(class),
        }
    }
}

pub fn panel<'a>(snapshot: &Snapshot, state: &'a UiState) -> Element<'a, Message> {
    let mut content = column![
        text("Local supervisor").size(18),
        text(format!(
            "{} attention · {} complete · {} failed · {} collisions",
            snapshot.count(SignalClass::Attention),
            snapshot.count(SignalClass::Completion),
            snapshot.count(SignalClass::Failure),
            snapshot.count(SignalClass::Collision),
        ))
        .size(12),
    ]
    .spacing(6);

    if snapshot.claims().is_empty() {
        content = content.push(text("No current supervisor signals.").size(12));
    }
    for claim in snapshot.claims() {
        content = content.push(text(format!(
            "{} · workspace {} · {}",
            claim.class().label(),
            claim.workspace_id(),
            claim.text()
        )));
        for evidence in claim.evidence().iter().take(3) {
            content = content.push(text(format!("↳ {}", evidence.label())).size(11));
        }
        if claim.evidence().len() > 3 {
            content =
                content.push(text(format!("↳ +{} more", claim.evidence().len() - 3)).size(11));
        }
    }
    for recommendation in snapshot.recommendations() {
        content = content.push(
            text(format!(
                "Next for workspace {}: {}",
                recommendation.workspace_id(),
                recommendation.text()
            ))
            .size(12),
        );
    }

    content = content.push(text("Desktop notifications").size(14));
    for class in SignalClass::ALL {
        let rule = state.notifications.rule(class);
        content = content.push(
            row![
                button(if rule.enabled { "On" } else { "Off" })
                    .on_press(Message::ToggleNotifications(class)),
                text(class.label()),
                button(text(format_cooldown(rule.cooldown.as_secs())))
                    .on_press(Message::CycleCooldown(class)),
            ]
            .spacing(6),
        );
    }
    content.into()
}

fn format_cooldown(seconds: u64) -> String {
    match seconds {
        0 => "no cooldown".to_owned(),
        1..=59 => format!("{seconds}s"),
        _ => format!("{}m", seconds / 60),
    }
}
