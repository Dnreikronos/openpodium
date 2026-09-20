use std::cell::Cell;

use notify_rust::Notification;
use openpodium::timeline::NavigationTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRequest {
    pub target: NavigationTarget,
    pub title: String,
    pub body: String,
}

pub async fn show(request: NotificationRequest) -> Option<NavigationTarget> {
    tokio::task::spawn_blocking(move || {
        let handle = Notification::new()
            .appname("OpenPodium")
            .summary(&request.title)
            .body(&request.body)
            .action("default", "Inspect")
            .show()
            .ok()?;
        let activated = Cell::new(false);
        handle.wait_for_action(|action| activated.set(is_activation(action)));
        activated.get().then_some(request.target)
    })
    .await
    .ok()
    .flatten()
}

fn is_activation(action: &str) -> bool {
    matches!(action, "default" | "inspect")
}

#[cfg(test)]
mod tests {
    use super::is_activation;

    #[test]
    fn only_notification_activation_opens_the_target() {
        assert!(is_activation("default"));
        assert!(is_activation("inspect"));
        assert!(!is_activation("__closed"));
    }
}
