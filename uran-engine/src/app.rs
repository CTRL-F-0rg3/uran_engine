use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes},
};
use uran_core::WindowDescriptor;
use uran_render::Renderer;
use uran_ecs::World;

pub struct App {
    descriptor: WindowDescriptor,
    pub world: World, // <-- Nasz świat ECS
}

impl App {
    pub fn new() -> Self {
        Self {
            descriptor: WindowDescriptor::default(),
            world: World::new(),
        }
    }

    pub fn window(mut self, descriptor: WindowDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    // Prosty sposób na dodanie systemu (na razie jako closure dla prostoty)
    // W przyszłości zrobimy to bardziej typowo (jak w Bevy)
    pub fn add_system(mut self, system: impl FnMut(&mut World) + 'static) -> Self {
        // Na tym etapie po prostu zapiszemy system w AppState. 
        // Dla uproszczenia kodu, zaimplementujemy to bezpośrednio w AppState poniżej.
        // (Pełna implementacja rejestracji systemów wymagałaby wektora funkcji, zrobimy to w v2)
        self
    }

    pub fn run(mut self) {
        let event_loop = EventLoop::new().unwrap();
        event_loop.set_control_flow(ControlFlow::Poll);

        let mut app_state = AppState {
            descriptor: self.descriptor,
            window: None,
            renderer: None,
            world: self.world, // Przekazujemy świat do stanu aplikacji
        };

        event_loop.run_app(&mut app_state).unwrap();
    }
}

struct AppState {
    descriptor: WindowDescriptor,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    world: World, // <-- Stan świata jest trzymany tutaj
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = WindowAttributes::default()
                .with_title(&self.descriptor.title)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.descriptor.width,
                    self.descriptor.height,
                ))
                .with_resizable(self.descriptor.resizable);
            
            let window = event_loop.create_window(attrs).unwrap();
            let window = Arc::new(window);

            let clear_color = self.descriptor.clear_color;
            let window_clone = window.clone();
            
            let renderer = pollster::block_on(Renderer::new(window_clone, clear_color));
            
            self.renderer = Some(renderer);
            self.window = Some(window);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(physical_size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(physical_size);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                // 1. Uruchom systemy (ruch)
                self.run_systems();

                // 2. Przekaż świat do renderera!
                if let Some(renderer) = &mut self.renderer {
                    renderer.render(&self.world); // <-- ZMIANA TUTAJ
                }
            }
            _ => {}
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl AppState {
    // System testowy - NA RAZIE WYŁĄCZONY
    fn run_systems(&mut self) {
        // for (_entity, transform) in self.world.query::<&mut uran_ecs::Transform>().iter() {
        //     transform.translation.x += 0.01;
        // }
    }
}