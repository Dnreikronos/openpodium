use std::cell::Cell;

use notify_rust::Notification;
use openpodium::timeline::NavigationTarget;

/// The bundle identifier every macOS notification is delivered under.
///
/// `mac-notification-sys` resolves its default by running the AppleScript
/// `get id of application "use_default"`. No application by that name exists,
/// so macOS answers with a modal "Choose Application" picker the first time a
/// notification is posted. Claiming the identifier up front consumes that
/// one-shot initialization, so the lookup never runs.
#[cfg(target_os = "macos")]
static NOTIFICATION_APPLICATION: std::sync::Once = std::sync::Once::new();

/// Picks the identifier notifications are delivered under.
///
/// A bundled build is launched through Launch Services, which exports its own
/// identifier; an unbundled development build has none, and falls back to the
/// same system bundle `mac-notification-sys` would have defaulted to.
#[cfg(target_os = "macos")]
fn notification_bundle_identifier(launch_services_identifier: Option<String>) -> String {
    launch_services_identifier
        .filter(|identifier| !identifier.trim().is_empty())
        .unwrap_or_else(|| "com.apple.Finder".to_owned())
}

#[cfg(target_os = "macos")]
fn claim_notification_application() {
    NOTIFICATION_APPLICATION.call_once(|| {
        let identifier = notification_bundle_identifier(std::env::var("__CFBundleIdentifier").ok());
        // A failure still counts: the library's own initialization guard is
        // spent either way, which is what keeps the picker from appearing.
        let _ = notify_rust::set_application(&identifier);
    });
}

#[cfg(not(target_os = "macos"))]
fn claim_notification_application() {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRequest {
    pub target: Option<NavigationTarget>,
    pub title: String,
    pub body: String,
}

pub async fn show(request: NotificationRequest) -> Option<NavigationTarget> {
    tokio::task::spawn_blocking(move || {
        claim_notification_application();
        let mut notification = Notification::new();
        notification
            .appname("OpenPodium")
            .summary(&request.title)
            .body(&request.body);
        if request.target.is_some() {
            notification.action("default", "Inspect");
        }
        let handle = notification.show().ok()?;
        let activated = Cell::new(false);
        handle.wait_for_action(|action| activated.set(is_activation(action)));
        activated.get().then_some(request.target).flatten()
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

    /// Guards the fix for the macOS "Choose Application" picker: the
    /// identifier must always be a real bundle, never an absent or blank one
    /// that sends the notification backend looking for `use_default`.
    #[cfg(target_os = "macos")]
    #[test]
    fn notifications_always_claim_a_resolvable_bundle_identifier() {
        use super::notification_bundle_identifier;

        assert_eq!(
            notification_bundle_identifier(Some("tech.mother.openpodium".to_owned())),
            "tech.mother.openpodium"
        );
        assert_eq!(
            notification_bundle_identifier(None),
            "com.apple.Finder",
            "an unbundled build still needs an identifier macOS can resolve"
        );
        assert_eq!(
            notification_bundle_identifier(Some("  ".to_owned())),
            "com.apple.Finder"
        );
    }
}
