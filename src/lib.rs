#[macro_use]
pub mod libretro;
mod retrolog;

use libc::c_char;

use std::path::{Path, PathBuf};
use std::fs::{File, metadata};
use std::io::Read;

use pockystation::{MASTER_CLOCK_HZ};
use pockystation::cpu::Cpu;
use pockystation::interrupt::Interrupt;
use pockystation::dac;
use pockystation::dac::Dac;
use pockystation::memory::Interconnect;
use pockystation::memory::bios::{Bios, BIOS_SIZE};
use pockystation::memory::flash::{Flash, FLASH_SIZE};

#[macro_use]
extern crate log;
extern crate libc;
#[macro_use]
extern crate pockystation;

/// Static system information sent to the frontend on request
const SYSTEM_INFO: libretro::SystemInfo = libretro::SystemInfo {
    library_name: cstring!("Pockystation"),
    library_version: VERSION_CSTR as *const _ as *const c_char,
    valid_extensions: cstring!("mcr"),
    need_fullpath: false,
    block_extract: false,
};

const SYSTEM_AV_INFO: libretro::SystemAvInfo = libretro::SystemAvInfo {
    geometry: libretro::GameGeometry {
        base_width: 32,
        base_height: 32,
        max_width: 32,
        max_height: 32,
        aspect_ratio: 1./1.,
    },
    timing: libretro::SystemTiming {
        fps: 60.,
        sample_rate: dac::SAMPLE_RATE_HZ as f64,
    }
};

pub const VERSION_CSTR: &'static str = concat!(env!("CARGO_PKG_VERSION"), '\0');

struct Context {
    cpu: Cpu,
}

impl Context {
    fn new(flash: &Path) -> Result<Context, ()> {

        if !libretro::set_pixel_format(libretro::PixelFormat::Xrgb8888) {
            error!("Can't set pixel format to XRGB 8888");
            return Err(());
        }

        let cpu = try!(Context::load(flash));

        Ok(Context {
            cpu: cpu,
        })
    }

    fn load(memory_card: &Path) -> Result<Cpu, ()> {

        let flash =
            match Context::load_flash(memory_card) {
                Some(f) => f,
                None => {
                    error!("Couldn't load flash memory, bailing out");
                    return Err(())
                }
            };

        let bios =
            match Context::find_bios() {
                Some(c) => c,
                None => {
                    error!("Couldn't find a bios, bailing out");
                    return Err(())
                }
            };

        let dac = Dac::new(Box::new(AudioBackend::new()));

        let inter = Interconnect::new(bios, flash, dac);

        Ok(Cpu::new(inter))
    }

    fn load_flash(path: &Path) -> Option<Flash> {
        match metadata(path) {
            Ok(md) => {
                if md.len() == FLASH_SIZE as u64 {
                    let mut file =
                        match File::open(path) {
                            Ok(f) => f,
                            Err(e) => {
                                warn!("Can't open {:?}: {}", path, e);
                                return None;
                            }
                        };

                    // Load the flash
                    let mut data = vec![0; FLASH_SIZE as usize];

                    if let Err(e) = file.read_exact(&mut data) {
                        warn!("Error while reading {:?}: {}", path, e);
                        return None;
                    }

                    match Flash::new(&data) {
                        Some(flash) => {
                            info!("Loaded flash memory from {:?}", path);
                            Some(flash)
                        }
                        None => {
                            debug!("Failed to load {:?}", path);
                            None
                        }
                    }
                } else {
                    error!("Invalid flash memory length (expected {}, got {})",
                           FLASH_SIZE, md.len());
                    None
                }
            }
            Err(e) => {
                error!("Can't get file size for {:?}: {}", path, e);
                None
            }
        }
    }

    /// Attempt to find the PocketStation BIOS in the system
    /// directory
    fn find_bios() -> Option<Bios> {
        let system_directory =
            match libretro::get_system_directory() {
                Some(dir) => dir,
                None => {
                    error!("The frontend didn't give us a system directory, \
                            no BIOS can be loaded");
                    return None;
                }
            };

        let dir =
            match ::std::fs::read_dir(&system_directory) {
                Ok(d) => d,
                Err(e) => {
                    error!("Can't read directory {:?}: {}",
                           system_directory, e);
                    return None;
                }
            };

        for entry in dir {
            match entry {
                Ok(entry) => {
                    let path = entry.path();

                    match entry.metadata() {
                        Ok(md) => {
                            if !md.is_file() {
                                debug!("Ignoring {:?}: not a file", path);
                            } else if md.len() != BIOS_SIZE as u64 {
                                debug!("Ignoring {:?}: bad size", path);
                            } else {
                                let bios = Context::try_bios(&path);

                                if bios.is_some() {
                                    // Found a valid BIOS!
                                    return bios;
                                }
                            }
                        }
                        Err(e) =>
                            warn!("Ignoring {:?}: can't get file metadata: {}",
                                  path, e)
                    }
                }
                Err(e) => warn!("Error while reading directory: {}", e),
            }
        }

        None
    }

    /// Attempt to read and load the BIOS at `path`
    fn try_bios(path: &Path) -> Option<Bios> {
        let mut file =
            match File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    warn!("Can't open {:?}: {}", path, e);
                    return None;
                }
            };

        // Load the BIOS
        let mut data = vec![0; BIOS_SIZE as usize];

        if let Err(e) = file.read_exact(&mut data) {
            warn!("Error while reading {:?}: {}", path, e);
            return None;
        }

        match Bios::new(&data) {
            Some(bios) => {
                info!("Using BIOS {:?}", path);
                Some(bios)
            }
            None => {
                debug!("Ignoring {:?}: not a known PocketStation BIOS", path);
                None
            }
        }
    }

    fn poll_controllers(&mut self) {
        let irq_controller = self.cpu.interconnect_mut().irq_controller_mut();

        for &(retrobutton, irq) in &BUTTON_MAP {
            let active =
                if libretro::button_pressed(0, retrobutton) {
                    true
                } else {
                    false
                };

            irq_controller.set_raw_interrupt(irq, active);
        }
    }
}

impl libretro::Context for Context {

    fn render_frame(&mut self) {
        self.poll_controllers();

        // Step for 1/60th of a second
        self.cpu.run_ticks(MASTER_CLOCK_HZ / 60);

        let fb = self.cpu.interconnect().lcd().framebuffer();

        let mut fb_out = [0u32; 32 * 32];

        for y in 0..32 {
            let row = fb[y];

            for x in 0..32 {
                if ((row >> x) & 1) == 0 {
                    fb_out[y * 32 + x] = 0xffffff;
                }
            }
        }

        libretro::frame_done(fb_out);
    }

    fn get_system_av_info(&self) -> libretro::SystemAvInfo {
        SYSTEM_AV_INFO
    }

    fn refresh_variables(&mut self) {
    }

    fn reset(&mut self) {
        self.cpu.reset();
    }

    fn gl_context_reset(&mut self) {
    }

    fn gl_context_destroy(&mut self) {
    }
}

/// Init function, guaranteed called only once (unlike `retro_init`)
fn init() {
    retrolog::init();
}

/// Called when a game is loaded and a new context must be built
fn load_game(memory: PathBuf) -> Option<Box<libretro::Context>> {
    info!("Loading {:?}", memory);

    Context::new(&memory).ok()
        .map(|c| Box::new(c) as Box<libretro::Context>)
}

libretro_variables!(
    struct CoreVariables (prefix = "pockystation") {
        _dummy: bool, _parse_bool
            => "Dummy option; disabled|enabled",
    });

fn _parse_bool(opt: &str) -> Result<bool, ()> {
    match opt {
        "true" | "enabled" | "on" => Ok(true),
        "false" | "disabled" | "off" => Ok(false),
        _ => Err(()),
    }
}

fn init_variables() {
    CoreVariables::register();
}

struct AudioBackend {
    /// Audio buffer. Libretro always assumes stereo so we'll have to
    /// duplicate each sample.
    buffer: [i16; 2048],
    /// Current position of the write pointer in the buffer
    pos: u16,
}

impl AudioBackend {
    fn new() -> AudioBackend {
        AudioBackend {
            buffer: [0; 2048],
            pos: 0,
        }
    }
}

impl dac::Backend for AudioBackend {
    fn push_sample(&mut self, sample: i16) {
        let pos = self.pos as usize;

        // Duplicate the sample for "stereo" output
        self.buffer[pos] = sample;
        self.buffer[pos + 1] = sample;

        self.pos += 2;

        if self.pos == self.buffer.len() as u16 {
            libretro::send_audio_samples(&self.buffer);
            self.pos = 0;
        }
    }
}

const BUTTON_MAP: [(libretro::JoyPadButton, Interrupt); 5] =
    [(libretro::JoyPadButton::A,     Interrupt::ActionButton),
     (libretro::JoyPadButton::Up,    Interrupt::UpButton),
     (libretro::JoyPadButton::Down,  Interrupt::DownButton),
     (libretro::JoyPadButton::Left,  Interrupt::LeftButton),
     (libretro::JoyPadButton::Right, Interrupt::RightButton)];
