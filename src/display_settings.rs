//! Startup graphics choices: which GPU API draws the window and whether to
//! trade vsync for lower pen-to-screen latency. They apply when the window is
//! created, so they live in their own small file read before startup instead
//! of the app preferences, which load after the window exists.
use crate::egui;
use eframe::{egui_wgpu, wgpu};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Renderer {
    /// OpenGL: the most widely supported choice and the fallback.
    #[default]
    OpenGl,
    /// wgpu picks the best available modern API (DirectX 12 or Vulkan).
    Automatic,
    Vulkan,
    DirectX12,
}

impl Renderer {
    pub const ALL: [Renderer; 4] = [
        Renderer::OpenGl,
        Renderer::Automatic,
        Renderer::Vulkan,
        Renderer::DirectX12,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Renderer::OpenGl => "OpenGL (most compatible)",
            Renderer::Automatic => "Automatic (DirectX 12 or Vulkan)",
            Renderer::Vulkan => "Vulkan",
            Renderer::DirectX12 => "DirectX 12",
        }
    }
    fn backends(self) -> Option<wgpu::Backends> {
        match self {
            Renderer::OpenGl => None,
            Renderer::Automatic => Some(wgpu::Backends::PRIMARY),
            Renderer::Vulkan => Some(wgpu::Backends::VULKAN),
            Renderer::DirectX12 => Some(wgpu::Backends::DX12),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    pub renderer: Renderer,
    /// Present frames without waiting for vsync and queue at most one frame.
    /// Lower pen-to-screen latency; may tear on some displays.
    pub low_latency: bool,
}

impl Default for DisplaySettings {
    /// Low latency is on by default: with vsync and two queued frames the line
    /// visibly trailed the pen on the reference tablet (2026-10-28). A saved
    /// choice, including turning it off, is kept.
    fn default() -> Self {
        Self {
            renderer: Renderer::default(),
            low_latency: true,
        }
    }
}

impl DisplaySettings {
    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("org", "Dudel", "Dudel")
            .map(|dirs| dirs.data_local_dir().join("display.json"))
    }
    /// Saved settings, or defaults when missing or unreadable.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    /// `DUDEL_RENDERER=opengl|auto|vulkan|dx12` overrides the renderer for
    /// one run without saving, for troubleshooting and capture checks.
    pub fn with_env_override(mut self) -> Self {
        if let Ok(value) = std::env::var("DUDEL_RENDERER") {
            self.renderer = match value.to_ascii_lowercase().as_str() {
                "opengl" | "gl" => Renderer::OpenGl,
                "auto" => Renderer::Automatic,
                "vulkan" => Renderer::Vulkan,
                "dx12" | "directx12" => Renderer::DirectX12,
                _ => self.renderer,
            };
        }
        self
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("User data directory unavailable")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::storage::atomic_write(&path, &bytes)
    }
    /// Window options for these settings on top of `base`.
    pub fn native_options(&self, base: eframe::NativeOptions) -> eframe::NativeOptions {
        self.native_options_for(base, wayland_session())
    }
    fn native_options_for(
        &self,
        mut base: eframe::NativeOptions,
        wayland: bool,
    ) -> eframe::NativeOptions {
        // Wayland compositors never tear, and Mesa's vsynced present waits for
        // frame callbacks that hidden windows never get: the UI thread would
        // block, miss compositor pings and be killed as unresponsive.
        let vsync = !self.low_latency && !wayland;
        match self.renderer.backends() {
            None => {
                base.renderer = eframe::Renderer::Glow;
                base.vsync = vsync;
            }
            Some(backends) => {
                base.renderer = eframe::Renderer::Wgpu;
                base.wgpu_options = egui_wgpu::WgpuConfiguration {
                    present_mode: if !vsync {
                        // Mailbox where supported (no tearing), else immediate.
                        wgpu::PresentMode::AutoNoVsync
                    } else {
                        wgpu::PresentMode::AutoVsync
                    },
                    desired_maximum_frame_latency: Some(if self.low_latency { 1 } else { 2 }),
                    wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
                        instance_descriptor: wgpu::InstanceDescriptor {
                            backends,
                            ..egui_wgpu::WgpuSetupCreateNew::default().instance_descriptor
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                };
            }
        }
        base
    }
}

/// winit prefers Wayland whenever a Wayland display is advertised.
fn wayland_session() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var_os("WAYLAND_SOCKET").is_some())
}

/// What actually drew the window this session, for the settings window.
pub fn active_description(cc: &eframe::CreationContext<'_>) -> String {
    match &cc.wgpu_render_state {
        Some(state) => {
            let info = state.adapter.get_info();
            format!("{:?} on {}", info.backend, info.name)
        }
        None => "OpenGL".into(),
    }
}

impl crate::Dudel {
    pub(crate) fn display_settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.display_window;
        egui::Window::new("Display")
            .open(&mut open)
            .collapsible(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(format!("Running now: {}", self.display_active));
                ui.separator();
                let mut settings = self.display_settings;
                ui.label("Graphics API");
                for renderer in Renderer::ALL {
                    ui.radio_value(&mut settings.renderer, renderer, renderer.name());
                }
                ui.checkbox(&mut settings.low_latency, "Low latency")
                    .on_hover_text("Shows strokes sooner by not waiting for vsync. Some displays may tear.");
                ui.label(
                    egui::RichText::new(
                        "Applies the next time Dudel starts. If the chosen API cannot start on this PC, Dudel falls back to OpenGL.",
                    )
                    .small()
                    .weak(),
                );
                if settings != self.display_settings {
                    self.display_settings = settings;
                    self.status = match settings.save() {
                        Ok(()) => "Display settings saved; restart Dudel to apply".into(),
                        Err(error) => format!("Display settings not saved: {error}"),
                    };
                }
            });
        self.display_window = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_choose_renderer_and_present_mode() {
        let base = eframe::NativeOptions::default;
        let gl = DisplaySettings {
            renderer: Renderer::OpenGl,
            low_latency: true,
        }
        .native_options_for(base(), false);
        assert_eq!(gl.renderer, eframe::Renderer::Glow);
        assert!(!gl.vsync);
        let vulkan = DisplaySettings {
            renderer: Renderer::Vulkan,
            low_latency: false,
        }
        .native_options_for(base(), false);
        assert_eq!(vulkan.renderer, eframe::Renderer::Wgpu);
        assert_eq!(
            vulkan.wgpu_options.present_mode,
            wgpu::PresentMode::AutoVsync
        );
        let egui_wgpu::WgpuSetup::CreateNew(setup) = &vulkan.wgpu_options.wgpu_setup else {
            panic!("expected a new wgpu setup");
        };
        assert_eq!(setup.instance_descriptor.backends, wgpu::Backends::VULKAN);
        let fast = DisplaySettings {
            renderer: Renderer::DirectX12,
            low_latency: true,
        }
        .native_options_for(base(), false);
        assert_eq!(
            fast.wgpu_options.present_mode,
            wgpu::PresentMode::AutoNoVsync
        );
        assert_eq!(fast.wgpu_options.desired_maximum_frame_latency, Some(1));
        // Wayland never waits for vsync, so hidden windows cannot block.
        let wayland = DisplaySettings::default().native_options_for(base(), true);
        assert!(!wayland.vsync);
        let wayland = DisplaySettings {
            renderer: Renderer::Vulkan,
            low_latency: false,
        }
        .native_options_for(base(), true);
        assert_eq!(
            wayland.wgpu_options.present_mode,
            wgpu::PresentMode::AutoNoVsync
        );
        assert_eq!(wayland.wgpu_options.desired_maximum_frame_latency, Some(2));
        // Old or partial files fall back to defaults field by field; low latency
        // is the default, while a saved "off" is respected.
        let partial: DisplaySettings = serde_json::from_str(r#"{"renderer":"Vulkan"}"#).unwrap();
        assert_eq!(
            partial,
            DisplaySettings {
                renderer: Renderer::Vulkan,
                low_latency: true
            }
        );
        let off: DisplaySettings = serde_json::from_str(r#"{"low_latency":false}"#).unwrap();
        assert!(!off.low_latency);
        assert!(
            !DisplaySettings::default()
                .native_options_for(base(), false)
                .vsync
        );
    }
}
