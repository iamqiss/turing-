use dioxus::prelude::*;
use crate::api::ActivityItem;

#[component]
pub fn ActivityFeed(items: Vec<ActivityItem>) -> Element {
    rsx! {
        div {
            for item in items {
                div { style: "padding:12px 0;border-bottom:1px solid var(--tt-border);",
                    "{item.actor} {item.action} {item.target}"
                }
            }
        }
    }
}
