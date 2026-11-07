#![allow(non_snake_case)]

use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use jvmti_bindings::sys::{jni, jvmti};
use lunar_shared::{Config, Mode, Status};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
	CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
	SetWindowsHookExW, UnhookWindowsHookEx, MSLLHOOKSTRUCT, MSG, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP,
};

#[link(name = "kernel32")]
extern "system" {
	fn CreateThread(
		attributes: *const c_void, stack_size: usize,
		start: Option<unsafe extern "system" fn(*mut c_void) -> u32>,
		argument: *mut c_void, flags: u32, thread_id: *mut u32,
	) -> isize;
}

#[repr(C)]
struct MouseInput {
	dx: i32,
	dy: i32,
	mouse_data: u32,
	flags: u32,
	time: u32,
	extra_info: usize,
}

#[repr(C)]
struct Input {
	kind: u32,
	mouse: MouseInput,
}

#[link(name = "user32")]
extern "system" {
	fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
}

const LEFT_DOWN: u32 = 0x0002;
const LEFT_UP: u32 = 0x0004;
const RIGHT_DOWN: u32 = 0x0008;
const RIGHT_UP: u32 = 0x0010;
const JVMTI_VERSION_1_2: i32 = 0x30010200;
const _: [(); 40] = [(); std::mem::size_of::<Input>()];

static DLL_MODULE: AtomicIsize = AtomicIsize::new(0);
static PHYSICAL_LEFT: AtomicBool = AtomicBool::new(false);
static MOUSE_HOOK_READY: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub unsafe extern "system" fn DllMain(module: *mut c_void, reason: u32, _: *mut c_void) -> i32 {
	if reason == 1 {
		DLL_MODULE.store(module as isize, Ordering::Relaxed);
		let thread = CreateThread(ptr::null(), 0, Some(worker), ptr::null_mut(), 0, ptr::null_mut());
		if thread != 0 { CloseHandle(thread); }
	}
	1
}

unsafe extern "system" fn worker(_: *mut c_void) -> u32 {
	let pid = GetCurrentProcessId();
	let result = catch_unwind(AssertUnwindSafe(|| run(pid)));
	let error = match result {
		Ok(Ok(())) => return 0,
		Ok(Err(error)) => error,
		Err(_) => "Bridge worker panicked".into(),
	};
	write_status(&Status { pid, ready: false, error, ..Status::default() });
	0
}

fn write_status(status: &Status) {
	if let Ok(data) = serde_json::to_vec(status) {
		let _ = lunar_shared::write_atomic(&lunar_shared::status_path(status.pid), &data);
	}
}

unsafe fn get_vm() -> Result<*mut jni::JavaVM, String> {
	let module: Vec<u16> = "jvm.dll".encode_utf16().chain(std::iter::once(0)).collect();
	let handle = GetModuleHandleW(module.as_ptr());
	if handle == 0 {
		return Err("jvm.dll is not loaded".into());
	}
	let address = GetProcAddress(handle, b"JNI_GetCreatedJavaVMs\0".as_ptr())
		.ok_or("JNI_GetCreatedJavaVMs not found")?;
	type GetVMs = unsafe extern "system" fn(*mut *mut jni::JavaVM, i32, *mut i32) -> i32;
	let get_vms: GetVMs = std::mem::transmute(address);
	let mut vm = ptr::null_mut();
	let mut count = 0i32;
	if get_vms(&mut vm, 1, &mut count) != 0 || count == 0 || vm.is_null() {
		return Err("No running Java VM was found".into());
	}
	Ok(vm)
}

struct Jvm {
	env: *mut jni::JNIEnv,
	ti: *mut jvmti::jvmtiEnv,
	vm: *mut jni::JavaVM,
}

impl Drop for Jvm {
	fn drop(&mut self) {
		unsafe { ((**self.vm).DetachCurrentThread)(self.vm); }
	}
}

impl Jvm {
	unsafe fn attach() -> Result<Self, String> {
		let vm = get_vm()?;
		let mut env: *mut jni::JNIEnv = ptr::null_mut();
		let rc = ((**vm).AttachCurrentThreadAsDaemon)(vm, &mut env as *mut _ as *mut *mut c_void, ptr::null_mut());
		if rc != 0 || env.is_null() {
			return Err(format!("AttachCurrentThreadAsDaemon failed ({rc})"));
		}
		let mut ti: *mut jvmti::jvmtiEnv = ptr::null_mut();
		let rc = ((**vm).GetEnv)(vm, &mut ti as *mut _ as *mut *mut c_void, JVMTI_VERSION_1_2);
		if rc != 0 || ti.is_null() {
			((**vm).DetachCurrentThread)(vm);
			return Err(format!("GetEnv(JVMTI) failed ({rc})"));
		}
		Ok(Self { env, ti, vm })
	}

	unsafe fn api(&self) -> &jni::JNINativeInterface_ {
		&**self.env
	}

	unsafe fn ti_api(&self) -> &jvmti::jvmtiInterface_1_ {
		&*(*self.ti).functions
	}

	unsafe fn clear(&self) {
		if (self.api().ExceptionCheck)(self.env) != 0 {
			(self.api().ExceptionClear)(self.env);
		}
	}

	unsafe fn dealloc(&self, pointer: *mut c_void) {
		if !pointer.is_null() {
			if let Some(free) = self.ti_api().Deallocate {
				free(self.ti, pointer as *mut u8);
			}
		}
	}

	unsafe fn take_string(&self, pointer: *mut c_char) -> String {
		if pointer.is_null() {
			return String::new();
		}
		let value = CStr::from_ptr(pointer).to_string_lossy().into_owned();
		self.dealloc(pointer as *mut c_void);
		value
	}

	unsafe fn signature(&self, class: jni::jclass) -> String {
		let mut signature = ptr::null_mut();
		let get = self.ti_api().GetClassSignature.unwrap();
		if get(self.ti, class, &mut signature, ptr::null_mut()) != jvmti::JVMTI_ERROR_NONE {
			return String::new();
		}
		self.take_string(signature)
	}

	unsafe fn fields(&self, class: jni::jclass) -> Vec<Field> {
		let mut count = 0;
		let mut array = ptr::null_mut();
		let get = self.ti_api().GetClassFields.unwrap();
		if get(self.ti, class, &mut count, &mut array) != jvmti::JVMTI_ERROR_NONE || array.is_null() {
			return Vec::new();
		}
		let mut result = Vec::new();
		for &id in std::slice::from_raw_parts(array, count as usize) {
			let mut name = ptr::null_mut();
			let mut signature = ptr::null_mut();
			if self.ti_api().GetFieldName.unwrap()(self.ti, class, id, &mut name, &mut signature, ptr::null_mut()) != jvmti::JVMTI_ERROR_NONE {
				self.dealloc(name as *mut c_void);
				self.dealloc(signature as *mut c_void);
				continue;
			}
			let mut modifiers = 0;
			self.ti_api().GetFieldModifiers.unwrap()(self.ti, class, id, &mut modifiers);
			result.push(Field {
				id, name: self.take_string(name), signature: self.take_string(signature),
				is_static: modifiers & 0x8 != 0,
			});
		}
		self.dealloc(array as *mut c_void);
		result
	}

	unsafe fn field_in_hierarchy(&self, class: jni::jclass, names: &[&str]) -> Option<jni::jfieldID> {
		let mut current = class;
		while !current.is_null() {
			if let Some(field) = self.fields(current).into_iter().find(|f| names.contains(&f.name.as_str())) {
				if current != class { (self.api().DeleteLocalRef)(self.env, current); }
				return Some(field.id);
			}
			let parent = (self.api().GetSuperclass)(self.env, current);
			if current != class { (self.api().DeleteLocalRef)(self.env, current); }
			current = parent;
		}
		None
	}

	unsafe fn method_in_hierarchy(
		&self, class: jni::jclass, names: &[&str], signature_matches: fn(&str) -> bool, is_static: bool,
	) -> Option<(jni::jclass, jni::jmethodID)> {
		let mut current = class;
		while !current.is_null() {
			let mut count = 0;
			let mut array = ptr::null_mut();
			if self.ti_api().GetClassMethods.unwrap()(self.ti, current, &mut count, &mut array) == jvmti::JVMTI_ERROR_NONE && !array.is_null() {
				for &id in std::slice::from_raw_parts(array, count as usize) {
					let mut name = ptr::null_mut();
					let mut sig = ptr::null_mut();
					if self.ti_api().GetMethodName.unwrap()(self.ti, id, &mut name, &mut sig, ptr::null_mut()) == jvmti::JVMTI_ERROR_NONE {
						let text = self.take_string(name);
						let signature = self.take_string(sig);
						let mut modifiers = 0;
						let found = self.ti_api().GetMethodModifiers.unwrap()(self.ti, id, &mut modifiers) == jvmti::JVMTI_ERROR_NONE;
						if found && names.contains(&text.as_str()) && signature_matches(&signature) && (modifiers & 0x8 != 0) == is_static {
							self.dealloc(array as *mut c_void);
							return Some((current, id));
						}
					} else {
						self.dealloc(name as *mut c_void);
						self.dealloc(sig as *mut c_void);
					}
				}
				self.dealloc(array as *mut c_void);
			}
			let parent = (self.api().GetSuperclass)(self.env, current);
			if current != class { (self.api().DeleteLocalRef)(self.env, current); }
			current = parent;
		}
		None
	}

	unsafe fn game_class(&self) -> Option<jni::jclass> {
		let mut count = 0;
		let mut classes = ptr::null_mut();
		if self.ti_api().GetLoadedClasses.unwrap()(self.ti, &mut count, &mut classes) != jvmti::JVMTI_ERROR_NONE || classes.is_null() {
			return None;
		}
		let mut found = ptr::null_mut();
		for &class in std::slice::from_raw_parts(classes, count as usize) {
			let signature = self.signature(class);
			if signature == "Lnet/minecraft/client/Minecraft;" || signature == "Lave;" {
				found = (self.api().NewLocalRef)(self.env, class) as jni::jclass;
				break;
			}
		}
		for &class in std::slice::from_raw_parts(classes, count as usize) {
			(self.api().DeleteLocalRef)(self.env, class);
		}
		self.dealloc(classes as *mut c_void);
		if found.is_null() { None } else { Some(found) }
	}

	unsafe fn enum_name(&self, value: jni::jobject) -> String {
		let class = (self.api().GetObjectClass)(self.env, value);
		if class.is_null() { self.clear(); return String::new(); }
		let name_method = (self.api().GetMethodID)(self.env, class, b"name\0".as_ptr() as *const c_char,
			b"()Ljava/lang/String;\0".as_ptr() as *const c_char);
		let name = if !name_method.is_null() {
			(self.api().CallObjectMethodA)(self.env, value, name_method, ptr::null())
		} else { ptr::null_mut() };
		self.clear();
		(self.api().DeleteLocalRef)(self.env, class);
		if name.is_null() { return String::new(); }
		let characters = (self.api().GetStringUTFChars)(self.env, name as jni::jstring, ptr::null_mut());
		let text = if characters.is_null() { String::new() }
			else { CStr::from_ptr(characters).to_string_lossy().into_owned() };
		if !characters.is_null() {
			(self.api().ReleaseStringUTFChars)(self.env, name as jni::jstring, characters);
		}
		(self.api().DeleteLocalRef)(self.env, name);
		text
	}
}

struct Field {
	id: jni::jfieldID,
	name: String,
	signature: String,
	is_static: bool,
}

struct Mapping {
	instance: jni::jobject,
	counter: jni::jfieldID,
	ray: jni::jfieldID,
	player: jni::jfieldID,
	screen: jni::jfieldID,
	world: jni::jfieldID,
	hit_type: Option<jni::jfieldID>,
	last_hit_search: Option<Instant>,
	held: Option<HeldMapping>,
}

impl Mapping {
	unsafe fn find(jvm: &Jvm) -> Result<Self, String> {
		let mc = jvm.game_class().ok_or("Minecraft 1.8.9 class not found")?;
		let fields = jvm.fields(mc);
		let mc_sig = jvm.signature(mc);
		let singleton = fields.iter().find(|f| f.is_static &&
			(f.name == "theMinecraft" || f.name == "field_71432_P" || f.signature == mc_sig))
			.ok_or("Minecraft instance field not found")?;
		let instance = (jvm.api().GetStaticObjectField)(jvm.env, mc, singleton.id);
		if instance.is_null() {
			jvm.clear();
			return Err("Minecraft has not initialized".into());
		}
		let select = |names: &[&str]| fields.iter().find(|f| !f.is_static && names.contains(&f.name.as_str())).map(|f| f.id);
		let counter = select(&["leftClickCounter", "field_71429_W"])
			.ok_or("leftClickCounter mapping unavailable")?;
		let ray = select(&["objectMouseOver", "field_71476_x"])
			.ok_or("objectMouseOver mapping unavailable")?;
		let player = select(&["thePlayer", "field_71439_g"])
			.ok_or("player mapping unavailable")?;
		let screen = select(&["currentScreen", "field_71462_r"])
			.ok_or("currentScreen mapping unavailable")?;
		let world = select(&["theWorld", "field_71441_e"])
			.ok_or("theWorld mapping unavailable")?;
		let instance = (jvm.api().NewGlobalRef)(jvm.env, instance);
		(jvm.api().DeleteLocalRef)(jvm.env, mc);
		if instance.is_null() { return Err("Cannot retain Minecraft instance".into()); }
		Ok(Self { instance, counter, ray, player, screen, world,
			hit_type: None, last_hit_search: None, held: None })
	}

	unsafe fn sample(&mut self, jvm: &Jvm, pid: u32, hit_fix: bool) -> Status {
		let mut status = Status { pid, ready: true, ..Status::default() };
		if (jvm.api().PushLocalFrame)(jvm.env, 64) != 0 {
			status.error = "JNI local frame failed".into();
			return status;
		}
		let screen = (jvm.api().GetObjectField)(jvm.env, self.instance, self.screen);
		status.gui_open = !screen.is_null();
		let world = (jvm.api().GetObjectField)(jvm.env, self.instance, self.world);
		let player = (jvm.api().GetObjectField)(jvm.env, self.instance, self.player);
		status.in_world = !world.is_null() && !player.is_null();
		if status.in_world && !status.gui_open {
			let ray = (jvm.api().GetObjectField)(jvm.env, self.instance, self.ray);
			if !ray.is_null() {
				if self.hit_type.is_none() && self.last_hit_search.is_none_or(|time| time.elapsed() >= Duration::from_secs(2)) {
					self.last_hit_search = Some(Instant::now());
					let class = (jvm.api().GetObjectClass)(jvm.env, ray);
					self.hit_type = jvm.field_in_hierarchy(class, &["typeOfHit", "field_72313_a"]);
				}
				if let Some(field) = self.hit_type {
					let kind = (jvm.api().GetObjectField)(jvm.env, ray, field);
					if !kind.is_null() {
						let name = jvm.enum_name(kind);
						status.target_known = matches!(name.as_str(), "BLOCK" | "MISS" | "ENTITY");
						status.targeting_block = name == "BLOCK";
						if hit_fix && name == "MISS" {
							let counter = (jvm.api().GetIntField)(jvm.env, self.instance, self.counter);
							if counter > 0 {
								(jvm.api().SetIntField)(jvm.env, self.instance, self.counter, 0);
							}
						}
					}
				}
			}
			if self.held.is_none() {
				self.held = HeldMapping::find(jvm, player);
			}
			if let Some(held) = &mut self.held {
				if let Some(item) = held.read(jvm, player) {
					status.held_known = true;
					status.held_item = item;
				}
			}
		}
		let exception = (jvm.api().ExceptionCheck)(jvm.env) != 0;
		if exception { jvm.clear(); }
		status.mapped = self.hit_type.is_some() && self.held.is_some()
			&& status.target_known && status.held_known && !exception;
		if exception {
			status.error = "JNI state inspection failed".into();
		} else if !status.mapped && status.in_world && !status.gui_open {
			status.error = "Waiting for Minecraft mappings or valid game state".into();
		}
		(jvm.api().PopLocalFrame)(jvm.env, ptr::null_mut());
		status
	}

}

struct HeldMapping {
	inventory: jni::jfieldID,
	current: jni::jfieldID,
	main: jni::jfieldID,
	item: Option<ItemMapping>,
	last_item_search: Option<Instant>,
}

struct ItemMapping {
	get_item: jni::jmethodID,
	get_id: jni::jmethodID,
	item_class: jni::jclass,
}

impl HeldMapping {
	unsafe fn find(jvm: &Jvm, player: jni::jobject) -> Option<Self> {
		let player_class = (jvm.api().GetObjectClass)(jvm.env, player);
		let inventory_field = jvm.field_in_hierarchy(player_class, &["inventory", "field_71071_by"])?;
		let inventory = (jvm.api().GetObjectField)(jvm.env, player, inventory_field);
		if inventory.is_null() { return None; }
		let inventory_class = (jvm.api().GetObjectClass)(jvm.env, inventory);
		let current = jvm.field_in_hierarchy(inventory_class, &["currentItem", "field_70461_c"])?;
		let main = jvm.field_in_hierarchy(inventory_class, &["mainInventory", "field_70462_a"])?;
		Some(Self { inventory: inventory_field, current, main, item: None, last_item_search: None })
	}

	unsafe fn read(&mut self, jvm: &Jvm, player: jni::jobject) -> Option<String> {
		let inventory = (jvm.api().GetObjectField)(jvm.env, player, self.inventory);
		if inventory.is_null() { return None; }
		let index = (jvm.api().GetIntField)(jvm.env, inventory, self.current);
		let array = (jvm.api().GetObjectField)(jvm.env, inventory, self.main) as jni::jobjectArray;
		if array.is_null() || !(0..9).contains(&index)
			|| index >= (jvm.api().GetArrayLength)(jvm.env, array as jni::jarray) {
			return None;
		}
		let stack = (jvm.api().GetObjectArrayElement)(jvm.env, array, index);
		if stack.is_null() { return Some("Hand".into()); }
		if self.item.is_none() && self.last_item_search.is_none_or(|time| time.elapsed() >= Duration::from_secs(2)) {
			self.last_item_search = Some(Instant::now());
			self.item = ItemMapping::find(jvm, stack);
		}
		let mapping = self.item.as_ref()?;
		let item = (jvm.api().CallObjectMethodA)(jvm.env, stack, mapping.get_item, ptr::null());
		if item.is_null() { return None; }
		let argument = jni::jvalue { l: item };
		let id = (jvm.api().CallStaticIntMethodA)(jvm.env, mapping.item_class, mapping.get_id, &argument);
		let result = match id {
			267 | 268 | 272 | 276 | 283 => "Sword",
			258 | 271 | 275 | 279 | 286 => "Axe",
			346 => "Fish",
			280 => "Stick",
			_ => "Other",
		};
		Some(result.into())
	}
}

impl ItemMapping {
	unsafe fn find(jvm: &Jvm, stack: jni::jobject) -> Option<Self> {
		let stack_class = (jvm.api().GetObjectClass)(jvm.env, stack);
		let (get_item_owner, get_item) = jvm.method_in_hierarchy(
			stack_class, &["getItem", "func_77973_b"],
			|sig| sig.starts_with("()L") && sig.ends_with(';'), false,
		)?;
		let item = (jvm.api().CallObjectMethodA)(jvm.env, stack, get_item, ptr::null());
		if get_item_owner != stack_class { (jvm.api().DeleteLocalRef)(jvm.env, get_item_owner); }
		if item.is_null() { return None; }
		let item_class = (jvm.api().GetObjectClass)(jvm.env, item);
		let (get_id_owner, get_id) = jvm.method_in_hierarchy(
			item_class, &["getIdFromItem", "func_150891_b"],
			|sig| sig.starts_with("(L") && sig.ends_with(";)I"), true,
		)?;
		let global = (jvm.api().NewGlobalRef)(jvm.env, get_id_owner) as jni::jclass;
		if global.is_null() { return None; }
		Some(Self { get_item, get_id, item_class: global })
	}
}

fn key_down(key: u16) -> bool {
	unsafe { GetAsyncKeyState(key as i32) as u16 & 0x8000 != 0 }
}

fn focused(pid: u32) -> bool {
	unsafe {
		let hwnd = GetForegroundWindow();
		if hwnd == 0 { return false; }
		let mut foreground_pid = 0u32;
		GetWindowThreadProcessId(hwnd, &mut foreground_pid);
		foreground_pid == pid
	}
}

fn send_mouse(first: u32, second: u32) {
	let input = |flags| Input {
		kind: 0,
		mouse: MouseInput { dx: 0, dy: 0, mouse_data: 0, flags, time: 0, extra_info: 0 },
	};
	let inputs = [input(first), input(second)];
	unsafe { SendInput(2, inputs.as_ptr(), std::mem::size_of::<Input>() as i32); }
}

unsafe extern "system" fn physical_mouse_hook(code: i32, message: usize, data: isize) -> isize {
	if code >= 0 && data != 0 {
		let mouse = &*(data as *const MSLLHOOKSTRUCT);
		if mouse.flags & 1 == 0 {
			if message as u32 == WM_LBUTTONDOWN {
				PHYSICAL_LEFT.store(true, Ordering::Relaxed);
			} else if message as u32 == WM_LBUTTONUP {
				PHYSICAL_LEFT.store(false, Ordering::Relaxed);
			}
		}
	}
	CallNextHookEx(0, code, message, data)
}

fn start_mouse_hook() {
	std::thread::spawn(|| unsafe {
		let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(physical_mouse_hook),
			DLL_MODULE.load(Ordering::Relaxed), 0);
		if hook == 0 { return; }
		MOUSE_HOOK_READY.store(true, Ordering::Release);
		let mut message: MSG = std::mem::zeroed();
		while GetMessageW(&mut message, 0, 0, 0) > 0 {}
		MOUSE_HOOK_READY.store(false, Ordering::Release);
		PHYSICAL_LEFT.store(false, Ordering::Release);
		UnhookWindowsHookEx(hook);
	});
}

struct Rng(u64);
impl Rng {
	fn new() -> Self {
		let seed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64;
		Self(seed | 1)
	}

	fn next(&mut self) -> u32 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 7;
		self.0 ^= self.0 << 17;
		self.0 as u32
	}

	fn cps(&mut self, config: &Config) -> u8 {
		match config.mode {
			Mode::Range => config.min_cps + (self.next() % (config.max_cps - config.min_cps + 1) as u32) as u8,
			Mode::Graph => {
				let mut ticket = self.next() % 100;
				for (index, weight) in config.graph.iter().enumerate() {
					if ticket < *weight as u32 { return 10 + index as u8; }
					ticket -= *weight as u32;
				}
				15
			}
		}
	}
}

fn run(pid: u32) -> Result<(), String> {
	let jvm = unsafe { Jvm::attach()? };
	start_mouse_hook();
	let mut mapping = None;
	let mut last_search = Instant::now() - Duration::from_secs(6);
	let mut last_sample = Instant::now() - Duration::from_millis(16);
	let mut last_file = Instant::now() - Duration::from_secs(2);
	let mut last_status = Instant::now() - Duration::from_secs(2);
	let mut config = lunar_shared::load_config();
	let mut status = Status { pid, ready: true, ..Status::default() };
	let mut rng = Rng::new();
	let mut next_left = Instant::now();
	let mut next_right = Instant::now();
	loop {
		if last_file.elapsed() >= Duration::from_millis(150) {
			config = lunar_shared::load_config();
			last_file = Instant::now();
		}
		if mapping.is_none() && last_search.elapsed() >= Duration::from_secs(5) {
			last_search = Instant::now();
			let discovered = unsafe {
				if (jvm.api().PushLocalFrame)(jvm.env, 64) != 0 {
					Err("Cannot create JNI local frame".to_owned())
				} else {
					let result = Mapping::find(&jvm);
					jvm.clear();
					(jvm.api().PopLocalFrame)(jvm.env, ptr::null_mut());
					result
				}
			};
			match discovered {
				Ok(found) => { mapping = Some(found); status.error.clear(); }
				Err(error) => status.error = error,
			}
		}
		if let Some(mapping) = &mut mapping {
			if last_sample.elapsed() >= Duration::from_millis(16) {
				status = unsafe { mapping.sample(&jvm, pid, config.hit_delay_fix) };
				last_sample = Instant::now();
			}
		}
		let can_click = status.mapped && status.in_world && !status.gui_open && status.target_known
			&& status.held_known && focused(pid)
			&& (!config.block_check || !status.targeting_block)
			&& lunar_shared::ITEMS.iter().enumerate().any(|(index, name)| config.items[index] && *name == status.held_item);
		if config.auto_clicker && can_click && MOUSE_HOOK_READY.load(Ordering::Acquire) {
			if PHYSICAL_LEFT.load(Ordering::Relaxed) {
				if Instant::now() >= next_left {
					send_mouse(LEFT_UP, LEFT_DOWN);
					if !PHYSICAL_LEFT.load(Ordering::Acquire) {
						send_mouse(LEFT_UP, LEFT_UP);
					}
					next_left = Instant::now() + Duration::from_secs_f64(1.0 / rng.cps(&config) as f64);
				}
			} else {
				next_left = Instant::now();
			}
			if config.activate_with_key && key_down(config.right_key) {
				if Instant::now() >= next_right {
					send_mouse(RIGHT_DOWN, RIGHT_UP);
					next_right = Instant::now() + Duration::from_secs_f64(1.0 / rng.cps(&config) as f64);
				}
			} else {
				next_right = Instant::now();
			}
		}
		if last_status.elapsed() >= Duration::from_millis(220) {
			if !MOUSE_HOOK_READY.load(Ordering::Acquire) && status.error.is_empty() {
				status.error = "Physical mouse hook is unavailable".into();
			}
			write_status(&status);
			last_status = Instant::now();
		}
		std::thread::sleep(Duration::from_millis(3));
	}
}
