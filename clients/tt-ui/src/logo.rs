use dioxus::prelude::*;

#[component]
pub fn TtMark(#[props(default = 36)] size: u32) -> Element {
    rsx! {
        svg {
            width: "{size}",
            height: "{size}",
            view_box: "0 0 64 64",
            xmlns: "http://www.w3.org/2000/svg",
            rect { x: "0", y: "0", width: "64", height: "64", rx: "14", fill: "#0a0a0a" }
            rect { x: "4", y: "4", width: "56", height: "56", rx: "11", fill: "#ff3c00" }
            path {
                d: "M13 21.5 C13 21.5 50.5 20.5 51 21.5",
                stroke: "#fafaf7", stroke_width: "7.2", stroke_linecap: "round", fill: "none",
            }
            path { d: "M23.5 21.5 L23.5 47.5", stroke: "#fafaf7", stroke_width: "6.4", stroke_linecap: "round" }
            path { d: "M40.5 21.5 L40.5 47.5", stroke: "#fafaf7", stroke_width: "6.4", stroke_linecap: "round" }
        }
    }
}
