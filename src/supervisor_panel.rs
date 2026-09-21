use iced::widget::{column, row, text};
use iced::{Alignment, Element, Fill};
use openpodium::supervisor::{NotificationSettings, SignalClass, Snapshot};

use crate::app::shell;
use crate::app::ui::{count_badge, rule, section, section_label};

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

pub fn panel<'a>(
    title: impl text::IntoFragment<'a>,
    snapshot: &Snapshot,
    state: &'a UiState,
) -> Element<'a, Message> {
    // The four signal counts read as a badge strip, so a glance is enough to
    // tell a healthy workspace from one that needs attention.
    let tally = |label: &'static str, count: usize, alarming: bool| {
        row![
            text(label).size(11).style(shell::muted_text),
            count_badge(count, alarming && count > 0),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
    };
    let mut content = column![
        row![
            tally("attention", snapshot.count(SignalClass::Attention), true),
            tally("done", snapshot.count(SignalClass::Completion), false),
            tally("failed", snapshot.count(SignalClass::Failure), true),
            tally("collisions", snapshot.count(SignalClass::Collision), true),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    ]
    .spacing(8);

    if snapshot.claims().is_empty() {
        content = content.push(
            text("No current supervisor signals.")
                .size(12)
                .style(shell::muted_text),
        );
    }
    for claim in snapshot.claims() {
        let mut entry = column![
            row![
                text(claim.class().label()).size(12),
                text(format!("workspace {}", claim.workspace_id()))
                    .size(11)
                    .style(shell::subtle_text),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            text(claim.text().to_string()).size(12),
        ]
        .spacing(2);
        for evidence in claim.evidence().iter().take(3) {
            entry = entry.push(
                text(format!("↳ {}", evidence.label()))
                    .size(11)
                    .style(shell::muted_text),
            );
        }
        if claim.evidence().len() > 3 {
            entry = entry.push(
                text(format!("↳ +{} more", claim.evidence().len() - 3))
                    .size(11)
                    .style(shell::subtle_text),
            );
        }
        content = content.push(entry);
    }
    for recommendation in snapshot.recommendations() {
        content = content.push(
            text(format!(
                "Next for workspace {}: {}",
                recommendation.workspace_id(),
                recommendation.text()
            ))
            .size(12)
            .style(shell::muted_text),
        );
    }

    content = content
        .push(rule())
        .push(section_label("Desktop notifications"));
    for class in SignalClass::ALL {
        let notification = state.notifications.rule(class);
        content = content.push(
            row![
                crate::app::ui::button(
                    text(if notification.enabled { "On" } else { "Off" }).size(12)
                )
                .padding([5, 12])
                .on_press(Message::ToggleNotifications(class)),
                text(class.label()).size(12).width(Fill),
                crate::app::ui::button(
                    text(format_cooldown(notification.cooldown.as_secs())).size(12)
                )
                .padding([5, 10])
                .on_press(Message::CycleCooldown(class)),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    section(title, content)
}

fn format_cooldown(seconds: u64) -> String {
    match seconds {
        0 => "no cooldown".to_owned(),
        1..=59 => format!("{seconds}s"),
        _ => format!("{}m", seconds / 60),
    }
}
