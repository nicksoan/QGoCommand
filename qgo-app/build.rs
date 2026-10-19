#[cfg(windows)]
fn main() {
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/QGoIcon.ico")
        .set("FileDescription", "QGo - Quick Launcher")
        .set("ProductName", "QGo")
        .set("InternalName", "QGo")
        .set("OriginalFilename", "qgo.exe");
    resource
        .compile()
        .expect("failed to compile Windows resources");
}

#[cfg(not(windows))]
fn main() {}
