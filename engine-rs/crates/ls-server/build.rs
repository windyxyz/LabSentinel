//! UX（2026-11-21）：Windows exe 图标嵌入（蓝盾 app-icon.ico，多尺寸 16-256）。
//! 仅 host=windows 时执行（交叉编译不跑 winresource）。

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        if let Err(e) = winresource::WindowsResource::new()
            .set_icon("app-icon.ico")
            .compile()
        {
            println!("cargo:warning=exe 图标嵌入失败（不影响功能）: {e}");
        }
    }
}
