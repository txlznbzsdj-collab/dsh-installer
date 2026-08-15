fn main() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/dsh-icon.ico");
        resource.set("ProductName", "DSH Installer");
        resource.set("FileDescription", "DeepSeek Harness 安装助手");
        resource
            .compile()
            .expect("failed to embed Windows resources");
    }
}
