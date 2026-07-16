pub mod scene;

use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering}
};

use bevy::{
	camera_controller::free_camera::FreeCameraPlugin,
	dev_tools::infinite_grid::InfiniteGridPlugin,
	log::LogPlugin,
	prelude::*,
	window::{ExitCondition, PrimaryWindow},
	winit::WinitPlugin
};
use bevy_mod_outline::OutlinePlugin;
use bevy_panorbit_camera::PanOrbitCameraPlugin;
use flume::{Receiver, Sender};
use parking_lot::RwLock;
use ttmap::TypeMap;

use crate::game;

pub struct SceneRenderer {
	game: Arc<RwLock<Option<Arc<game::EntitiesOverlay>>>>,
	is_active: Arc<AtomicBool>,
	channels: TypeMap
}

#[doc(hidden)]
#[macro_export]
macro_rules! __to_bevy {
	(
		$(#[$meta:meta])*
		struct $name:ident $($rest:tt)*
	) => {
		$(#[$meta])*
		pub struct $name $($rest)*

		inventory::submit!(crate::render::MessageInitialiser(|channels, app| {
			let (sender, receiver) = flume::unbounded::<$name>();
			channels.insert(sender);
			app.insert_resource(crate::render::BevyReceiver(receiver));
		}));
	};

	(
		$(#[$meta:meta])*
		enum $name:ident $($rest:tt)*
	) => {
		$(#[$meta])*
		pub enum $name $($rest)*

		inventory::submit!(crate::render::MessageInitialiser(|channels, app| {
			let (sender, receiver) = flume::unbounded::<$name>();
			channels.insert(sender);
			app.insert_resource(crate::render::BevyReceiver(receiver));
		}));
	};
}

pub use crate::__to_bevy as to_bevy;

#[doc(hidden)]
#[macro_export]
macro_rules! __from_bevy {
	(
		$(#[$meta:meta])*
		struct $name:ident $($rest:tt)*
	) => {
		$(#[$meta])*
		pub struct $name $($rest)*

		inventory::submit!(crate::render::MessageInitialiser(|channels, app| {
			let (sender, receiver) = flume::unbounded::<$name>();
			channels.insert(receiver);
			app.insert_resource(crate::render::BevySender(sender));
		}));
	};

	(
		$(#[$meta:meta])*
		enum $name:ident $($rest:tt)*
	) => {
		$(#[$meta])*
		pub enum $name $($rest)*

		inventory::submit!(crate::render::MessageInitialiser(|channels, app| {
			let (sender, receiver) = flume::unbounded::<$name>();
			channels.insert(receiver);
			app.insert_resource(crate::render::BevySender(sender));
		}));
	};
}

pub use crate::__from_bevy as from_bevy;

/// Messages to Bevy
mod to_bevy {
	super::to_bevy!(
		struct CreateWindow;
	);

	super::to_bevy!(
		struct Exit;
	);
}

/// Messages from Bevy
mod from_bevy {
	super::from_bevy!(
		struct AppExited;
	);
}

struct MessageInitialiser(fn(channels: &mut TypeMap, app: &mut App));

inventory::collect!(MessageInitialiser);

impl Default for SceneRenderer {
	fn default() -> Self {
		let is_active = Arc::new(AtomicBool::new(false));
		let game = Arc::new(RwLock::new(None));

		let (tx_channels, rx_channels) = flume::bounded(1);

		std::thread::spawn({
			let is_active = is_active.clone();
			let game = game.clone();
			move || {
				let (exited_sender, exited_receiver) = flume::unbounded::<from_bevy::AppExited>();

				let mut channels = TypeMap::new();
				channels.insert(exited_receiver);

				let mut app = App::new();

				for init in inventory::iter::<MessageInitialiser> {
					(init.0)(&mut channels, &mut app);
				}

				tx_channels.send(channels).unwrap();

				app.add_plugins(
					DefaultPlugins
						.set(WindowPlugin {
							primary_window: None,
							exit_condition: ExitCondition::DontExit,
							..default()
						})
						.set(WinitPlugin {
							run_on_any_thread: true,
							..default()
						})
						.build()
						.disable::<LogPlugin>()
				);

				app.add_plugins((
					#[cfg(feature = "bevy-inspector-egui")]
					(
						bevy_inspector_egui::bevy_egui::EguiPlugin::default(),
						bevy_inspector_egui::quick::WorldInspectorPlugin::new().run_if(
							|window_query: Query<&Window>| {
								if let Ok(window) = window_query.single() {
									window.visible
								} else {
									false
								}
							}
						),
						bevy::dev_tools::diagnostics_overlay::DiagnosticsOverlayPlugin,
						bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
						bevy::render::diagnostic::MeshAllocatorDiagnosticPlugin,
						bevy::pbr::diagnostic::MaterialAllocatorDiagnosticPlugin::<StandardMaterial>::default()
					),
					MeshPickingPlugin,
					FreeCameraPlugin,
					PanOrbitCameraPlugin,
					InfiniteGridPlugin,
					OutlinePlugin::JUMP_FLOOD,
					TransformGizmoPlugin
				))
				.insert_resource(IsActive(is_active.clone()))
				.insert_resource(GameFiles(game.clone()))
				.add_systems(Update, create_window_system)
				.add_systems(Update, window_closed_system)
				.add_systems(Update, exit_system);

				#[cfg(feature = "bevy-inspector-egui")]
				app.insert_resource(bevy_inspector_egui::bevy_egui::EguiGlobalSettings {
					auto_create_primary_context: false,
					..default()
				});

				scene::configure(&mut app);

				app.run();

				let _ = exited_sender.send(from_bevy::AppExited);
				is_active.store(false, Ordering::SeqCst);
			}
		});

		let channels = rx_channels.recv().unwrap();

		Self {
			game,
			is_active,
			channels
		}
	}
}

impl SceneRenderer {
	pub fn set_game(&self, game: Option<Arc<game::EntitiesOverlay>>) {
		*self.game.write() = game;
	}

	pub fn send<T: 'static + Send + Sync>(&self, message: T) {
		let _ = self.channels.get::<Sender<T>>().unwrap().send(message);
	}

	pub fn recv<T: 'static + Send + Sync>(&self) -> Result<T, flume::RecvError> {
		self.channels.get::<Receiver<T>>().unwrap().recv()
	}

	pub fn is_active(&self) -> bool {
		self.is_active.load(Ordering::SeqCst)
	}

	pub fn start(&self) {
		if !self.is_active.load(Ordering::SeqCst) {
			self.is_active.store(true, Ordering::SeqCst);
			self.send(to_bevy::CreateWindow);
		}
	}

	pub fn exit(&self) {
		self.send(to_bevy::Exit);
		let _ = self.recv::<from_bevy::AppExited>();
	}
}

#[derive(Resource)]
struct BevyReceiver<T>(Receiver<T>);

#[derive(Resource)]
struct BevySender<T>(Sender<T>);

#[derive(Resource)]
struct IsActive(Arc<AtomicBool>);

#[derive(Resource)]
struct GameFiles(Arc<RwLock<Option<Arc<game::EntitiesOverlay>>>>);

#[derive(Component, Clone, Copy, Default)]
struct Clearable;

fn create_window_system(
	mut commands: Commands,
	receiver: Res<BevyReceiver<to_bevy::CreateWindow>>,
	to_clear: Query<Entity, With<Clearable>>
) {
	if receiver.0.try_recv().is_ok() {
		for entity in to_clear.iter() {
			commands.entity(entity).try_despawn();
		}

		commands.spawn((
			Window {
				title: "GlacierKit - Scene".into(),
				..default()
			},
			PrimaryWindow
		));
	}
}

fn window_closed_system(
	mut commands: Commands,
	mut close_events: MessageReader<bevy::window::WindowClosed>,
	is_active: Res<IsActive>,
	to_clear: Query<Entity, With<Clearable>>
) {
	for _ in close_events.read() {
		for entity in to_clear.iter() {
			commands.entity(entity).try_despawn();
		}

		is_active.0.store(false, Ordering::SeqCst);
		scene::window_closed(&mut commands);
	}
}

fn exit_system(mut commands: Commands, receiver: Res<BevyReceiver<to_bevy::Exit>>) {
	if receiver.0.try_recv().is_ok() {
		commands.write_message(AppExit::Success);
	}
}
