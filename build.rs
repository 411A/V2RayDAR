fn main() {
    println!("cargo:rerun-if-changed=assets/V2RayDAR_logo_v1.ico");

    #[cfg(target_os = "windows")]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/V2RayDAR_logo_v1.ico");
        resource.set("FileDescription", "V2RayDAR");
        resource.set("ProductName", "V2RayDAR");
        resource.set("CompanyName", "V2RayDAR");
        resource
            .compile()
            .expect("failed to embed Windows executable resources");
    }
}
