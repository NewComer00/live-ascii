use std::error::Error;
use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::ptr;

use live_ascii::context::*;
use live_ascii::effect::pose::*;
use live_ascii::expression::manager::*;
use live_ascii::ffi::*;
use live_ascii::live::json::*;
use live_ascii::model_setting::ModelSetting;
use live_ascii::motion::manager::*;
use live_ascii::physics::{Physics, json::*};
use live_ascii::shader::*;
use live_ascii::tracker::*;

use live_ascii::renderer::*;
use live_ascii::utils::*;
use live_ascii::vts::{VtsConfig, VtsServer};

use clap::Parser;

#[derive(Parser, Debug)]
struct Args {
    model_setting: String, // model3.json file
    #[arg(short, long)]
    camera: bool,
    #[arg(short, long)]
    physics: bool,
    #[arg(short, long)]
    text_shader: Option<String>, // file path
    /// Image output protocol: "halfblock" (default), "sixel", "kitty"
    #[arg(long, default_value = "halfblock")]
    image_protocol: String,
    #[arg(long)]
    mouse: bool,
    /// Background color for transparent areas: "rgba(r,g,b,a)" (e.g. "rgba(0,0,0,0)"), not applied in sixel mode
    #[arg(long, default_value = "rgba(0,0,0,0)")]
    bg_color: String,
    /// View scale percentage, e.g. "200%" for 2x zoom (default "100%")
    #[arg(long, default_value = "100%")]
    scale: String,
    /// Horizontal view offset as percentage of panel width, e.g. "-10%" shifts left (default "0%")
    #[arg(long, default_value = "0%", allow_hyphen_values = true)]
    offsetx: String,
    /// Vertical view offset as percentage of panel height, e.g. "50%" shifts down (default "0%")
    #[arg(long, default_value = "0%", allow_hyphen_values = true)]
    offsety: String,
    /// Sixel quantette resolution (sixel mode only). 100% = reference (10×20 px/cell).
    /// Also accepts explicit px/cell, e.g. 4x8. Output is always reference display size. Default: 100%.
    #[arg(long, default_value = "100%")]
    sixel_resolution: String,
    /// Sixel palette / dithering (sixel mode only): low, medium, high, ultra, epic. Default: high.
    #[arg(long, default_value = "high")]
    sixel_color_quality: String,
    /// Enable VTube Studio-compatible WebSocket API server
    #[arg(long)]
    vts: bool,
    /// VTS API WebSocket port (default 8001)
    #[arg(long, default_value = "8001")]
    vts_port: u16,
    /// Auto-approve VTS plugin authentication token requests
    #[arg(long, default_value_t = true)]
    vts_auto_approve: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    // load model setting
    let mut model_setting = ModelSetting::new(&args.model_setting)?;
    let model3_path = Path::new(&args.model_setting).canonicalize()?;
    let base_dir = model3_path.parent().unwrap();

    let file_refs = &model_setting.file_references;

    // get model name
    let name;

    let mut moc_data = Vec::new();
    if let Some(moc_relative_path) = &file_refs.moc {
        let full_moc_path = base_dir.join(moc_relative_path);
        let mut file = File::open(&full_moc_path)?;
        name = get_file_name(moc_relative_path.to_str().unwrap());
        file.read_to_end(&mut moc_data)?;
    } else {
        panic!("MOC path not found in JSON");
    }

    // loading moc3
    let moc_mem = unsafe {
        let mem = allocate_aligned(moc_data.len(), CSM_ALIGNOF_MOC);
        ptr::copy_nonoverlapping(moc_data.as_ptr(), mem, moc_data.len());

        // check moc3 consistency
        let consistency = csmHasMocConsistency(mem as *mut c_void, moc_data.len() as u32);

        if consistency == 0 {
            panic!("The moc3 file is malformed.");
        }

        csmReviveMocInPlace(mem as *mut c_void, moc_data.len() as u32)
    };

    // create a model from moc3
    let model_ptr = unsafe {
        let size = csmGetSizeofModel(moc_mem);
        let mem = allocate_aligned(size as usize, CSM_ALIGNOF_MODEL);
        csmInitializeModelInPlace(moc_mem, mem as *mut c_void, size)
    };

    // load texture
    let mut textures = vec![];
    for relative_path in &file_refs.textures {
        let full_path = base_dir.join(relative_path);
        if full_path.is_file() {
            let texture = image::open(&full_path)?.to_rgba8();
            let enhanced = live_ascii::utils::enhance_edges(&texture);
            textures.push(image::DynamicImage::ImageRgba8(enhanced));
        }
    }
    // initalize tracker
    let tracker = Tracker::new();

    // initialize terminal
    let mut context = Context::new(
        false,
        model_setting.clone(),
        base_dir.to_str().unwrap(),
        args.camera,
        tracker,
    );
    if args.camera {
        context.tracker.run()?;
    }
    if args.physics {
        context.use_physics = true;
    }
    context.image_protocol = match args.image_protocol.to_lowercase().as_str() {
        "sixel" => ImageProtocol::Sixel,
        "kitty" => ImageProtocol::Kitty,
        _ => ImageProtocol::HalfBlock,
    };
    context.sixel_resolution = parse_sixel_resolution(&args.sixel_resolution);
    context.sixel_color_quality = parse_sixel_color_quality(&args.sixel_color_quality);
    context.mouse = args.mouse;
    context.bg_color = parse_bg_color(&args.bg_color);

    let mut shader_manager = ShaderManager::new();
    if let Some(path) = &args.text_shader {
        let content = fs::read_to_string(path)
            .expect(&format!("Failed to read {}.", path))
            .replace([' ', '\t', '\n', '\r'], "");
        shader_manager.insert_hd(Shader::Text(content.into()));
    }

    // load live json
    let full_path = Path::new(base_dir).join(&format!("{}.live.json", name));
    if let Ok(data) = fs::read_to_string(&full_path) {
        if let Ok(live) = Live::from_data(data) {
            context.set_live_setting(live);
        } else {
            panic!(
                "Error: The parameters or format of {:?} has error.",
                full_path
            );
        }
    }

    if args.vts {
        let vts = VtsServer::start(
            VtsConfig {
                port: args.vts_port,
                auto_approve: args.vts_auto_approve,
                model_name: name.to_string(),
            },
            &model_setting,
            context.live_setting.as_ref(),
        );
        context.vts = Some(vts);
    }

    // initialize motion manager
    let mut mm = MotionManager::new();

    // initialize renderer
    let mut renderer = Renderer::new(model_ptr, textures, shader_manager);

    // apply startup view transform from CLI flags
    renderer.apply_startup_transform(
        parse_percent(&args.scale, 100.0) / 100.0,
        parse_percent(&args.offsetx, 0.0) / 100.0,
        parse_percent(&args.offsety, 0.0) / 100.0,
    );

    // initialize expression
    let mut em = ExpressionManager::new();

    let mut pos = if let Some(pose_file) = model_setting.get_pose_file_name() {
        Some(Pose::from_path(
            base_dir.to_str().unwrap(),
            pose_file.to_str().unwrap(),
        )?)
    } else {
        None
    };

    // physics

    let mut ph_file = if let Some(physics_file) = model_setting.get_physics_file_name() {
        let p_json =
            PhysicsJson::from_path(base_dir.to_str().unwrap(), physics_file.to_str().unwrap())?;
        Some(Physics::from_json(p_json))
    } else {
        None
    };

    renderer.render(
        &mut context,
        &mut mm,
        &mut model_setting,
        &mut em,
        &mut pos,
        &mut ph_file,
    )?;

    Ok(())
}

fn parse_bg_color(input: &str) -> (u8, u8, u8, u8) {
    let fail = (0, 0, 0, 0);
    let s = input.strip_prefix("rgba(").unwrap_or(input);
    let s = s.strip_suffix(')').unwrap_or(s);
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    if parts.len() != 4 {
        eprintln!("Invalid --bg-color format '{}', expected rgba(r,g,b,a). Using transparent.", input);
        return fail;
    }
    let parse = |s: &str| -> Option<u8> {
        s.parse::<u16>().ok().filter(|&v| v <= 255).map(|v| v as u8)
    };
    let r = parse(parts[0]).unwrap_or_else(|| {
        eprintln!("Invalid --bg-color red component '{}', using transparent.", parts[0]);
        0
    });
    let g = parse(parts[1]).unwrap_or_else(|| {
        eprintln!("Invalid --bg-color green component '{}', using transparent.", parts[1]);
        0
    });
    let b = parse(parts[2]).unwrap_or_else(|| {
        eprintln!("Invalid --bg-color blue component '{}', using transparent.", parts[2]);
        0
    });
    let a = parse(parts[3]).unwrap_or_else(|| {
        eprintln!("Invalid --bg-color alpha component '{}', using transparent.", parts[3]);
        0
    });
    (r, g, b, a)
}

fn parse_percent(input: &str, default: f32) -> f32 {
    let s = input.strip_suffix('%').unwrap_or(input);
    s.parse::<f32>().unwrap_or_else(|_| {
        eprintln!("Invalid percent value '{}', using default {:.0}%.", input, default);
        default
    })
}
