#[cfg(windows)]
fn main() {
	let mut res = winres::WindowsResource::new();
	res.set_icon("assets/icon.ico");
	res.set("CompanyName", "SimSool");
	res.set("LegalCopyright", "Author: SimSool");
	res.set("ProductName", "Lunar Injector");
	res.set("FileDescription", "Lunar Client 1.8.9 Injector & Utility Tool");
	if let Err(e) = res.compile() {
		eprintln!("Warning: failed to compile Windows resources: {e}");
	}
}

#[cfg(not(windows))]
fn main() {}