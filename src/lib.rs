#[macro_use]
pub mod libretro;
mod retrolog;
mod savestate;

use std::path::{Path, PathBuf};
use std::fs::{File, metadata};
use std::io::Read;

use libc::c_char;

use rustc_serialize::{Encodable, Decodable};

use pockystation::{MASTER_CLOCK_HZ};
use pockystation::cpu::Cpu;
use pockystation::interrupt::Interrupt;
use pockystation::dac;
use pockystation::dac::Dac;
use pockystation::rtc::Bcd;
use pockystation::memory::{Interconnect, Byte};
use pockystation::memory::bios::{Bios, BIOS_SIZE};
use pockystation::memory::flash::{Flash, FLASH_SIZE};

#[macro_use]
extern crate log;
extern crate libc;
#[macro_use]
extern crate pockystation;
extern crate time;
extern crate rustc_serialize;

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
    /// Pockystation CPU instance holding all the emulated state
    cpu: Cpu,
    /// If true the emulated RTC is periodically synchronized with the
    /// host clock.
    rtc_host_sync: bool,
    /// If true the emulator will rotate the display when the software
    /// requests it
    lcd_rotation_en: bool,
    /// Countdown for RTC synchronization with host if
    /// `rtc_host_sync` is true. Decreases by one every frame,
    /// synchronizes when it reaches 0.
    rtc_sync_counter: u32,
    /// Cached value for the maximum savestate size in bytes
    savestate_max_len: usize,
}

impl Context {
    fn new(flash: &Path) -> Result<Context, ()> {

        if !libretro::set_pixel_format(libretro::PixelFormat::Xrgb8888) {
            error!("Can't set pixel format to XRGB 8888");
            return Err(());
        }

        let cpu = try!(Context::load(flash));

        let mut context = Context {
            cpu: cpu,
            lcd_rotation_en: true,
            rtc_host_sync: false,
            rtc_sync_counter: 0,
            savestate_max_len: 0,
        };

        libretro::Context::refresh_variables(&mut context);

        let max_len = try!(context.compute_savestate_max_length());

        context.savestate_max_len = max_len;

        Ok(context)
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
                    error!("Couldn't find a BIOS, bailing out");
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

    fn compute_savestate_max_length(&mut self) -> Result<usize, ()> {
        // In order to get the full size we're just going to use a
        // dummy Write struct which will just count how many bytes are
        // being written
        struct WriteCounter(usize);

        impl ::std::io::Write for WriteCounter {
            fn write(&mut self, buf: &[u8]) -> ::std::io::Result<usize> {
                let len = buf.len();

                self.0 += len;

                Ok(len)
            }

            fn flush(&mut self) -> ::std::io::Result<()> {
                Ok(())
            }
        }

        let mut counter = WriteCounter(0);

        try!(self.save_state(&mut counter));

        let len = counter.0;

        // Our savestate format has variable length so let's add a bit of headroom
        let len = len + 1024;

        Ok(len)
    }

    fn save_state(&self, writer: &mut ::std::io::Write) -> Result<(), ()> {

        let mut encoder =
            match savestate::Encoder::new(writer) {
                Ok(encoder) => encoder,
                Err(e) => {
                    warn!("Couldn't create savestate encoder: {:?}", e);
                    return Err(())
                }
            };

        match self.cpu.encode(&mut encoder) {
            Ok(_) => Ok(()),
            Err(e) => {
                warn!("Couldn't serialize emulator state: {:?}", e);
                Err(())
            }
        }
    }

    fn load_state(&mut self, reader: &mut ::std::io::Read) -> Result<(), ()> {
        let mut decoder =
            match savestate::Decoder::new(reader) {
                Ok(decoder) => decoder,
                Err(e) => {
                    warn!("Couldn't create savestate decoder: {:?}", e);
                    return Err(())
                }
            };

        let mut cpu: Cpu =
            match Decodable::decode(&mut decoder) {
                Ok(cpu) => cpu,
                Err(e) => {
                    warn!("Couldn't decode savestate: {:?}", e);
                    return Err(())
                }
            };

        let bios =
            match Context::find_bios() {
                Some(c) => c,
                None => {
                    error!("Couldn't find a BIOS, bailing out");
                    return Err(())
                }
            };

        let flash = self.cpu.interconnect().flash().data().clone();

        cpu.interconnect_mut().set_bios(bios);
        cpu.interconnect_mut().flash_mut().set_data(flash);
        cpu.interconnect_mut().dac_mut().set_backend(Box::new(AudioBackend::new()));

        self.cpu = cpu;

        Ok(())
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

    /// Synchronize emulated RTC with the host
    fn sync_host_rtc(&mut self) {
        let now = time::now();

        let inter = self.cpu.interconnect_mut();

        let year = now.tm_year + 1900;
        let century = (year / 100) as u8;
        let century = Bcd::from_binary(century).unwrap();
        let year = (year % 100) as u8;

        // The century is not stored in the RTC, it's stored in RAM at
        // address 0xcf. Hopefully this address is always correct...
        inter.store::<Byte>(0xcf, century.bcd() as u32);

        {
            let rtc = inter.rtc_mut();

            // Handle leap seconds, just in case...
            let secs =
                match now.tm_sec {
                    s @ 0...59 => s as u8,
                    _ => 59,
                };

            rtc.set_seconds(Bcd::from_binary(secs).unwrap());
            rtc.set_minutes(Bcd::from_binary(now.tm_min as u8).unwrap());
            rtc.set_hours(Bcd::from_binary(now.tm_hour as u8).unwrap());

            let week_day = now.tm_wday as u8 + 1;
            rtc.set_week_day(Bcd::from_binary(week_day).unwrap());

            let day = now.tm_mday as u8 + 1;
            rtc.set_day(Bcd::from_binary(day).unwrap());

            let month = now.tm_mon as u8 + 1;
            rtc.set_month(Bcd::from_binary(month).unwrap());

            rtc.set_year(Bcd::from_binary(year).unwrap());
        }
    }
}

impl libretro::Context for Context {

    fn render_frame(&mut self) {
        self.poll_controllers();

        if self.rtc_host_sync {
            if self.rtc_sync_counter == 0 {
                self.sync_host_rtc();
                self.rtc_sync_counter = RTC_SYNC_DELAY_FRAMES;
            }

            self.rtc_sync_counter -= 1;
        }

        // Step for 1/60th of a second
        self.cpu.run_ticks(MASTER_CLOCK_HZ / 60);

        let lcd = self.cpu.interconnect().lcd();

        let fb = lcd.framebuffer();

        let mut fb_out = [0u32; 32 * 32];

        let rotate = self.lcd_rotation_en && lcd.rotated();

        for y in 0..32 {
            let row = fb[y];

            for x in 0..32 {
                if ((row >> x) & 1) == 0 {
                    let mut off = y * 32 + x;

                    if rotate {
                        off = 32 * 32 - off - 1;
                    }

                    fb_out[off] = 0xffffff;
                }
            }
        }

        libretro::frame_done(fb_out);
    }

    fn get_system_av_info(&self) -> libretro::SystemAvInfo {
        SYSTEM_AV_INFO
    }

    fn refresh_variables(&mut self) {
        self.rtc_host_sync = CoreVariables::rtc_host_sync();
        self.lcd_rotation_en = CoreVariables::lcd_rotation_en();
    }

    fn reset(&mut self) {
        self.cpu.reset();
    }

    fn gl_context_reset(&mut self) {
    }

    fn gl_context_destroy(&mut self) {
    }

    fn serialize_size(&self) -> usize {
        self.savestate_max_len
    }

    fn serialize(&self, mut buf: &mut [u8]) -> Result<(), ()> {
        self.save_state(&mut buf)
    }

    fn unserialize(&mut self, mut buf: &[u8]) -> Result<(), ()> {
        self.load_state(&mut buf)
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
        rtc_host_sync: bool, parse_bool
            => "Synchronize real-time clock with host; disabled|enabled",
        lcd_rotation_en: bool, parse_bool
            => "Enable display rotation; enabled|disabled",
    });

fn parse_bool(opt: &str) -> Result<bool, ()> {
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

/// Number of frame elapsing between RTC synchronization (if the
/// option is enabled).
const RTC_SYNC_DELAY_FRAMES: u32 = 60;
