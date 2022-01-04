use clap::Parser;
use goxlr_usb::channels::Channel;
use goxlr_usb::faders::Fader;
use goxlr_usb::goxlr::GoXLR;
use simplelog::*;
use goxlr_usb::channelstate::ChannelState;
use goxlr_usb::buttonstate;
use goxlr_usb::buttonstate::{Buttons, ButtonStates};
use goxlr_usb::commands::Command::SetButtonStates;
use goxlr_usb::routing::{InputDevice, OutputDevice};

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    /// How verbose should the output be (can be repeated for super verbosity!)
    #[clap(short, long, parse(from_occurrences))]
    verbose: u8,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Args::parse();

    let (log_level, usb_debug) = match cli.verbose {
        0 => (LevelFilter::Warn, false),
        1 => (LevelFilter::Info, false),
        2 => (LevelFilter::Debug, false),
        3 => (LevelFilter::Debug, true),
        _ => (LevelFilter::Trace, true),
    };

    CombinedLogger::init(vec![TermLogger::new(
        log_level,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )])
    .unwrap();

    if usb_debug {
        goxlr_usb::rusb::set_log_level(goxlr_usb::rusb::LogLevel::Debug);
    }

    let mut goxlr = GoXLR::open()?;

    goxlr.set_volume(Channel::Mic, 0xFF)?;
    goxlr.set_volume(Channel::Chat, 0xFF)?;
    goxlr.set_volume(Channel::Music, 0xFF)?;
    goxlr.set_volume(Channel::System, 0xFF)?;

    goxlr.set_fader(Fader::A, Channel::Mic)?;
    goxlr.set_fader(Fader::B, Channel::Chat)?;
    goxlr.set_fader(Fader::C, Channel::Music)?;
    goxlr.set_fader(Fader::D, Channel::System)?;

    goxlr.set_channel_state(Channel::System, ChannelState::Unmuted);

    // IMPORTANT: THIS CODE ONLY WORKS WITH THE FULL GOXLR.
    // So this is a complex one, there's no direct way to retrieve the button colour states
    // directly from the GoXLR, it's all managed by the app.. So for testing, all we're going
    // to do here, is a simple example of managing the buttons.

    // Code will be left commented until GoXLR Mini support is added.

    /*
    // Define our buttons, set them all to a Dimmed State..
    let mut buttonStates : [u8;24] = [ButtonStates::Dimmed.id(); 24];

    // Now set 'Mute' to a lit state..
    buttonStates[Buttons::MicrophoneMute.position()] = ButtonStates::On.id();

    // Apply the state.
    goxlr.set_button_states(buttonStates);
    */

    /*
    Ok, this is awkward as hell, this *WILL* need improving, but proof-of-concept currently..

    Left and Right channels for both sources and destinations appear to be configured separately by
    the GoXLR, but it's essentially handled with a list of 'on' or 'off' for channels in the
    correct order. The defined 'on' value is 8192 as a u16, which as bytes and endiand is
    [0x00, 0x20], so I'm just slapping the 0x20 into the correct byte slot of the list, and
    sending it run through (correct byte position being provided by an enum for convenience)
     */

/*
    let mut gameRoutingStateLeft: [u8;22] = [0; 22];
    gameRoutingStateLeft[OutputDevice::HeadphonesLeft.position()] = 0x20;
    gameRoutingStateLeft[OutputDevice::BroadcastMixLeft.position()] = 0x20;
    goxlr.set_routing(InputDevice::GameLeft, gameRoutingStateLeft);


    let mut gameRoutingStateRight : [u8;22] = [0; 22];
    gameRoutingStateRight[OutputDevice::HeadphonesRight.position()] = 0x20;
    gameRoutingStateRight[OutputDevice::BroadcastMixRight.position()] = 0x20;
    goxlr.set_routing(InputDevice::GameRight, gameRoutingStateRight);
*/

    Ok(())
}
