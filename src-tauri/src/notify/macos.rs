//! Notifications through the UserNotifications framework. The notification
//! plugin posts through NSUserNotificationCenter, deprecated since macOS 11,
//! which on current macOS delivers nothing and never asks for permission.
#![allow(unsafe_code)]

use std::ptr::NonNull;
use std::sync::OnceLock;
use std::time::Duration;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{NSError, NSString, NSUUID};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use tauri::AppHandle;
use tracing::{info, warn};

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Only an app bundle may use the framework: an unbundled build (`cargo
/// run`) gets an exception from it.
pub fn available() -> bool {
    std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class does
    // not implement Drop.
    #[unsafe(super = NSObject)]
    #[name = "TowerBridgeNotificationDelegate"]
    struct Delegate;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: the method signatures match the protocol.
    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // macOS hides notifications of the app in front unless asked to show
        // them; "Send test" is pressed with the dashboard in front.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        // A click on a notification opens the dashboard.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            _response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            if let Some(app) = APP.get() {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || crate::window::show_dashboard(&handle));
            }
            completion.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's init on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }
}

fn describe(error: *mut NSError) -> Option<String> {
    // SAFETY: the framework passes null or a valid NSError.
    unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string())
}

/// Take over the notification center and ask for permission: macOS shows
/// its prompt the first time only, later calls return the earlier answer.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = Delegate::new();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The center keeps only a weak reference; the delegate lives as long as
    // the app.
    std::mem::forget(delegate);
    let answered = RcBlock::new(|granted: Bool, error: *mut NSError| match describe(error) {
        Some(e) => warn!("notification permission: {e}"),
        None => info!("notifications allowed: {}", granted.as_bool()),
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &answered,
    );
}

pub fn send(title: &str, body: &str) {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSUUID::new().UUIDString(),
        &content,
        None,
    );
    let done = RcBlock::new(|error: *mut NSError| {
        if let Some(e) = describe(error) {
            warn!("notification failed: {e}");
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

/// Whether notifications are turned off for the app in System Settings.
pub fn blocked() -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let got = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the framework passes valid settings for the call.
        let _ = tx.send(unsafe { settings.as_ref() }.authorizationStatus());
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&got);
    // The answer comes on another queue; never wait on it for long.
    rx.recv_timeout(Duration::from_secs(2)) == Ok(UNAuthorizationStatus::Denied)
}
