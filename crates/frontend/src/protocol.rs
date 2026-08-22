//! 自生成协议绑定:im-v2 与 vkb-v1 尚未进 wayland-protocols crate
//! (niri 公告 v2 但上游 staging 未合入 main),故 vendor XML 并用
//! wayland-scanner proc-macro 在编译期生成 client 绑定。
//!
//! text-input-v3 不需要在这里生成——wayland-protocols crate 的
//! `wp::text_input::zv3` 已有现成绑定。

#![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
#![allow(non_upper_case_globals, non_snake_case, unused_imports, missing_docs)]

pub mod input_method_v2 {
    pub mod client {
        //! im-v2 事件参数引用 zwp_text_input_v3 的枚举(change_cause /
        //! content_hint / content_purpose);生成代码在各接口子模块内按
        //! `super::zwp_text_input_v3` 寻址,re-export 到本层满足引用。
        pub mod zwp_text_input_v3 {
            pub use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::*;
        }

        use wayland_client;
        use wayland_client::protocol::*;

        pub mod __interfaces {
            use wayland_client::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("./protocols/input-method-unstable-v2.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("./protocols/input-method-unstable-v2.xml");
    }
}

pub mod virtual_keyboard_v1 {
    pub mod client {
        use wayland_client;
        use wayland_client::protocol::*;

        pub mod __interfaces {
            use wayland_client::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("./protocols/virtual-keyboard-unstable-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_client_code!("./protocols/virtual-keyboard-unstable-v1.xml");
    }
}
