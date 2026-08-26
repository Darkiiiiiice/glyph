//! glyph-frontend 库面:协议绑定 + 纯逻辑(渲染/配置)供 glyph(IME 本体)、
//! glyph-type(按键注入测试工具)与 examples(如 popup_preview)共用。

pub mod config;
pub mod protocol;
pub mod render;
