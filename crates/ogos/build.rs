#![allow(clippy::uninlined_format_args)]

fn main() {
    println!("cargo:rustc-link-search=./target/release");

    let mut win_res = winres::WindowsResource::new();
    win_res.set_manifest(r#"
        <?xml version="1.0" encoding="UTF-8" standalone="yes"?>
        <assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
            <assemblyIdentity
                name="Ogos"
                version="1.0.0.0"
                type="win32"
                processorArchitecture="*"
            />

            <application xmlns="urn:schemas-microsoft-com:asm.v3">
                <windowsSettings>
                    <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
                </windowsSettings>
            </application>
            <dependency>
                <dependentAssembly>
                    <assemblyIdentity
                        name="Microsoft.Windows.Common-Controls"
                        version="6.0.0.0"
                        type="win32"
                        processorArchitecture="*"
                        publicKeyToken="6595b64144ccf1df"
                        language="*"
                    />
                </dependentAssembly>
            </dependency>
        </assembly>
    "#);

    if let Err(err) = win_res.compile() {
        eprintln!("failed to compile manifest: {}", err);
        std::process::exit(1);
    }
}
