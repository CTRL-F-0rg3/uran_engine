use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes},
};
use uran_core::WindowDescriptor;
use uran_render::Renderer;

pub struct App {
    descriptor: WindowDescriptor,
}

impl App {
    pub fn new() -> Self {
        Self {
            descriptor: WindowDescriptor::default(),
        }
    }

    pub fn window(mut self, descriptor: WindowDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    pub fn run(self) {
        let event_loop = EventLoop::new().unwrap();
        event_loop.set_control_flow(ControlFlow::Poll); // Poller

        let mut app_state = AppState {
            descriptor: self.descriptor,
            window: None,
            renderer: None,
        };

        event_loop.run_app(&mut app_state).unwrap();
    }
}

struct AppState {
    descriptor: WindowDescriptor,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
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
            
            // Inicjalizacja wgpu (async) wewnątrz sync pętli
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
                if let Some(renderer) = &mut self.renderer {
                    renderer.render();
                }
            }
            _ => {}
        }

        // Wymuś ciągłe rysowanie (poller)
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}