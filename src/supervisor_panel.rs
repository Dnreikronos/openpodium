use iced::widget::{column, row, text};
use iced::{Alignment, Element, Fill};
use openpodium::supervisor::{NotificationSettings, SignalClass};

use crate::app::ui::section;

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

pub fn panel(state: &UiState) -> Element<'_, Message> {
    let mut content = column![].spacing(8);
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
    section("Desktop notifications", content)
}

fn format_cooldown(seconds: u64) -> String {
    match seconds {
        0 => "no cooldown".to_owned(),
        1..=59 => format!("{seconds}s"),
        _ => format!("{}m", seconds / 60),
    }
}
