use std::fs;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context as ErrorContext, Result};
use enum_map::EnumMap;
use log::{debug, warn};
use quick_xml::events::{BytesDecl, BytesStart, Event};
use quick_xml::{Reader, Writer};
use strum::EnumProperty;
use strum::IntoEnumIterator;
use zip::write::FileOptions;

use crate::components::animation::AnimationTree;
use crate::components::browser::BrowserPreviewTree;
use crate::components::context::Context;
use crate::components::echo::EchoEncoderBase;
use crate::components::effects::Effects;
use crate::components::fader::Fader;
use crate::components::gender::GenderEncoderBase;
use crate::components::hardtune::HardtuneEffectBase;
use crate::components::megaphone::MegaphoneEffectBase;
use crate::components::mixer::{InputChannels, Mixers, OutputChannels};
use crate::components::mute::MuteButton;
use crate::components::mute_chat::MuteChat;
use crate::components::pitch::PitchEncoderBase;
use crate::components::preset_writer::PresetWriter;
use crate::components::reverb::ReverbEncoderBase;
use crate::components::robot::RobotEffectBase;
use crate::components::root::RootElement;
use crate::components::sample::SampleBase;
use crate::components::scribble::Scribble;
use crate::components::simple::{SimpleElement, SimpleElements};
use crate::components::submix::mix_routing_tree::{Mix, MixRoutingTree};
use crate::components::submix::submixer::SubMixer;
use crate::SampleButtons::{BottomLeft, BottomRight, Clear, TopLeft, TopRight};
use crate::{Faders, Preset, SampleButtons};

#[derive(Debug)]
pub struct Profile {
    settings: ProfileSettings,
    scribbles: [Vec<u8>; 4],
}

#[derive(Debug)]
pub struct Attribute {
    pub(crate) name: String,
    pub(crate) value: String,
}

impl Profile {
    pub fn load<R: Read + std::io::Seek>(read: R) -> Result<Self> {
        debug!("Loading Profile Archive..");

        let mut archive = zip::ZipArchive::new(read)?;

        let mut scribbles: [Vec<u8>; 4] = Default::default();

        // Load the scribbles if they exist, store them in memory for later fuckery.
        for (i, scribble) in scribbles.iter_mut().enumerate() {
            let filename = format!("scribble{}.png", i + 1);
            if let Ok(mut file) = archive.by_name(filename.as_str()) {
                *scribble = vec![0; file.size() as usize];
                file.read_exact(scribble)?;
            }
        }

        debug!("Attempting to read profile.xml..");
        let result = ProfileSettings::load(archive.by_name("profile.xml")?);
        match result {
            Ok(settings) => Ok(Profile {
                settings,
                scribbles,
            }),
            Err(e) => {
                warn!("Unable to Load Profile: {}", e);
                bail!("Unable to Load Profile");
            }
        }
    }

    // Ok, this is better.
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let temp_file = tempfile::NamedTempFile::new()?;

        debug!("Creating Temporary Save File: {:?}", temp_file.path());

        // Create a new ZipFile at the requested location
        let mut archive = zip::ZipWriter::new(temp_file.as_file());

        // Store the profile..
        archive.start_file("profile.xml", FileOptions::default())?;
        self.settings.write_to(&mut archive)?;

        // Write the scribbles..
        for (i, scribble) in self.scribbles.iter().enumerate() {
            // Only write if there's actually data stored..
            if !self.scribbles[i].is_empty() {
                let filename = format!("scribble{}.png", i + 1);
                archive.start_file(filename, FileOptions::default())?;
                archive.write_all(scribble)?;
            }
        }
        archive.finish()?;

        // The archive has finished writing, we don't need it anymore (keeping it live prevents
        // us from removing the temporary file).
        drop(archive);

        // Syncing Write..
        temp_file.as_file().sync_all()?;

        // Once complete, we simply move the file over the existing file..
        debug!("Save Complete, copying to {:?}", path.as_ref());
        fs::copy(temp_file.as_ref(), path)?;

        debug!("Removing Temporary File: {:?}", temp_file);
        fs::remove_file(temp_file)?;
        Ok(())
    }

    pub fn save_preset(&self, path: impl AsRef<Path>) -> Result<()> {
        self.settings.write_preset(path)?;
        Ok(())
    }

    pub fn settings(&self) -> &ProfileSettings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut ProfileSettings {
        &mut self.settings
    }

    pub fn get_scribble(&self, id: usize) -> &Vec<u8> {
        &self.scribbles[id]
    }
}

#[derive(Debug)]
pub struct ProfileSettings {
    root: RootElement,
    browser: BrowserPreviewTree,
    animation_tree: AnimationTree,
    mix_routing: MixRoutingTree,
    submix_tree: SubMixer,
    mixer: Mixers,
    context: Context,
    mute_chat: MuteChat,
    mute_buttons: EnumMap<Faders, Option<MuteButton>>,
    faders: EnumMap<Faders, Option<Fader>>,
    effects: EnumMap<Preset, Option<Effects>>,
    scribbles: EnumMap<Faders, Option<Scribble>>,
    sampler_map: EnumMap<SampleButtons, Option<SampleBase>>,
    simple_elements: EnumMap<SimpleElements, Option<SimpleElement>>,
    megaphone_effect: MegaphoneEffectBase,
    robot_effect: RobotEffectBase,
    hardtune_effect: HardtuneEffectBase,
    reverb_encoder: ReverbEncoderBase,
    echo_encoder: EchoEncoderBase,
    pitch_encoder: PitchEncoderBase,
    gender_encoder: GenderEncoderBase,
}

impl ProfileSettings {
    pub fn load<R: Read>(read: R) -> Result<Self> {
        // Wrap our reader into a Buffered Reader for parsing..
        let buf_reader = BufReader::new(read);
        let mut reader = Reader::from_reader(buf_reader);

        debug!("Preparing Structure..");

        let mut root = RootElement::new();
        let mut browser = BrowserPreviewTree::new("browserPreviewTree".to_string());

        let mut animation_tree = AnimationTree::new("animationTree".to_string());

        let mut mix_routing = MixRoutingTree::new();
        let mut submix_tree = SubMixer::new();

        let mut mixer = Mixers::new();
        let mut context = Context::new("selectedContext".to_string());
        let mut mute_chat = MuteChat::new("muteChat".to_string());

        let mut mute_buttons: EnumMap<Faders, Option<MuteButton>> = EnumMap::default();
        let mut faders: EnumMap<Faders, Option<Fader>> = EnumMap::default();
        let mut scribbles: EnumMap<Faders, Option<Scribble>> = EnumMap::default();

        let mut effects: EnumMap<Preset, Option<Effects>> = EnumMap::default();

        let mut simple_elements: EnumMap<SimpleElements, Option<SimpleElement>> =
            Default::default();

        let mut megaphone_effect = MegaphoneEffectBase::new("megaphoneEffect".to_string());
        let mut robot_effect = RobotEffectBase::new("robotEffect".to_string());
        let mut hardtune_effect = HardtuneEffectBase::new("hardtuneEffect".to_string());
        let mut reverb_encoder = ReverbEncoderBase::new("reverbEncoder".to_string());
        let mut echo_encoder = EchoEncoderBase::new("echoEncoder".to_string());
        let mut pitch_encoder = PitchEncoderBase::new("pitchEncoder".to_string());
        let mut gender_encoder = GenderEncoderBase::new("genderEncoder".to_string());

        let mut sampler_map: EnumMap<SampleButtons, Option<SampleBase>> = EnumMap::default();
        let mut active_sample_button: Option<&mut SampleBase> = None;

        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                // Applies to most tags, represents a tag with no child
                Ok(Event::Empty(ref e)) => {
                    let (name, attributes) = wrap_start_event(e)?;
                    if name == "browserPreviewTree" {
                        browser.parse_browser(&attributes)?;
                        continue;
                    }

                    if name == "animationTree" {
                        animation_tree.parse_animation(&attributes)?;
                        continue;
                    }

                    if name == "mixRoutingTree" {
                        mix_routing.parse_mix_tree(&attributes)?;
                        continue;
                    }

                    if name == "monitorTree" {
                        submix_tree.parse_monitor(&attributes)?;
                        continue;
                    }

                    if name == "linkingTree" {
                        submix_tree.parse_linking(&attributes)?;
                        continue;
                    }

                    if name == "mixerTree" {
                        mixer.parse_mixers(&attributes)?;
                        continue;
                    }

                    if name == "selectedContext" {
                        context.parse_context(&attributes)?;
                        continue;
                    }

                    if name == "muteChat" {
                        mute_chat.parse_mute_chat(&attributes)?;
                        continue;
                    }

                    // Might need to pattern match this..
                    if name.starts_with("mute") && name != "muteChat" {
                        // In the XML, the count starts as 1, here, we're gonna store as 0.
                        if let Some(id) = name
                            .chars()
                            .last()
                            .map(|s| u8::from_str(&s.to_string()))
                            .transpose()?
                        {
                            let mut mute_button = MuteButton::new(id);
                            mute_button.parse_button(&attributes)?;
                            mute_buttons[Faders::iter().nth((id - 1).into()).unwrap()] =
                                Some(mute_button);
                            continue;
                        }
                    }

                    if name.starts_with("FaderMeter") {
                        // In the XML, the count starts at 0, and we have different capitalisation :D
                        if let Some(id) = name
                            .chars()
                            .last()
                            .map(|s| u8::from_str(&s.to_string()))
                            .transpose()?
                        {
                            let mut fader = Fader::new(id);
                            fader.parse_fader(&attributes)?;
                            faders[Faders::iter().nth(id.into()).unwrap()] = Some(fader);
                            continue;
                        }
                    }

                    if name.starts_with("effects") {
                        let mut found = false;

                        // Version 2, now with more enum, search for the prefix..
                        for preset in Preset::iter() {
                            if preset.get_str("contextTitle").unwrap() == name {
                                let mut effect = Effects::new(preset);
                                effect.parse_effect(&attributes)?;
                                effects[preset] = Some(effect);
                                found = true;
                                break;
                            }
                        }
                        if found {
                            continue;
                        }
                    }

                    if name.starts_with("scribble") {
                        if let Some(id) = name
                            .chars()
                            .last()
                            .map(|s| u8::from_str(&s.to_string()))
                            .transpose()?
                        {
                            let mut scribble = Scribble::new(id);
                            scribble.parse_scribble(&attributes)?;
                            scribbles[Faders::iter().nth((id - 1).into()).unwrap()] =
                                Some(scribble);
                            continue;
                        }
                    }

                    if name.starts_with("megaphoneEffectpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            megaphone_effect.parse_megaphone_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("robotEffectpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            robot_effect.parse_robot_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("hardtuneEffectpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            hardtune_effect.parse_hardtune_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("reverbEncoderpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            reverb_encoder.parse_reverb_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("echoEncoderpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            echo_encoder.parse_echo_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("pitchEncoderpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            pitch_encoder.parse_pitch_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("genderEncoderpreset") {
                        if let Ok(preset) = ProfileSettings::parse_preset(name.clone()) {
                            gender_encoder.parse_gender_preset(preset, &attributes)?;
                            continue;
                        }
                    }

                    if name.starts_with("sampleStack") {
                        if let Some(id) = name.chars().last() {
                            if let Some(button) = &mut active_sample_button {
                                button.parse_sample_stack(id, &attributes)?;
                                continue;
                            }
                        }
                    }

                    if name.starts_with("sampleBank")
                        || name == "fxClear"
                        || name == "swear"
                        || name == "globalColour"
                        || name == "logoX"
                    {
                        // In this case, the tag name, and attribute prefixes are the same..
                        let mut simple_element = SimpleElement::new(name.clone());
                        simple_element.parse_simple(&attributes)?;
                        simple_elements[SimpleElements::from_str(&name)?] = Some(simple_element);

                        continue;
                    }

                    if name == "AppTree" {
                        // This is handled by ValueTreeRoot
                        continue;
                    }

                    warn!("Unhandled Tag: {}", name);
                }

                // Represents a tag which has children
                Ok(Event::Start(ref e)) => {
                    let (name, attributes) = wrap_start_event(e)?;

                    if name == "ValueTreeRoot" {
                        // This also handles <AppTree, due to a single shared value.
                        root.parse_root(&attributes)?;

                        // This code was made for XML version 2, v1 not currently supported.
                        if root.get_version() > 3 {
                            bail!("Unsupported Profile Version {}", root.get_version());
                        }
                        continue;
                    }

                    if name == "submixerTree" {
                        submix_tree.parse_submixer(&attributes)?;
                        continue;
                    }

                    if name == "megaphoneEffect" {
                        megaphone_effect.parse_megaphone_root(&attributes)?;
                        continue;
                    }

                    if name == "robotEffect" {
                        robot_effect.parse_robot_root(&attributes)?;
                        continue;
                    }

                    if name == "hardtuneEffect" {
                        hardtune_effect.parse_hardtune_root(&attributes)?;
                        continue;
                    }

                    if name == "reverbEncoder" {
                        reverb_encoder.parse_reverb_root(&attributes)?;
                        continue;
                    }

                    if name == "echoEncoder" {
                        echo_encoder.parse_echo_root(&attributes)?;
                        continue;
                    }

                    if name == "pitchEncoder" {
                        pitch_encoder.parse_pitch_root(&attributes)?;
                        continue;
                    }

                    if name == "genderEncoder" {
                        gender_encoder.parse_gender_root(&attributes)?;
                        continue;
                    }

                    // These can probably be a little cleaner..
                    if name == "sampleTopLeft" {
                        let mut sampler = SampleBase::new(TopLeft);
                        sampler.parse_sample_root(&attributes)?;
                        sampler_map[TopLeft] = Some(sampler);
                        active_sample_button = sampler_map[TopLeft].as_mut();
                        continue;
                    }

                    if name == "sampleTopRight" {
                        let mut sampler = SampleBase::new(TopRight);
                        sampler.parse_sample_root(&attributes)?;
                        sampler_map[TopRight] = Some(sampler);
                        active_sample_button = sampler_map[TopRight].as_mut();
                        continue;
                    }

                    if name == "sampleBottomLeft" {
                        let mut sampler = SampleBase::new(BottomLeft);
                        sampler.parse_sample_root(&attributes)?;
                        sampler_map[BottomLeft] = Some(sampler);
                        active_sample_button = sampler_map[BottomLeft].as_mut();
                        continue;
                    }

                    if name == "sampleBottomRight" {
                        let mut sampler = SampleBase::new(BottomRight);
                        sampler.parse_sample_root(&attributes)?;
                        sampler_map[BottomRight] = Some(sampler);
                        active_sample_button = sampler_map[BottomRight].as_mut();
                        continue;
                    }

                    if name == "sampleClear" {
                        let mut sampler = SampleBase::new(Clear);
                        sampler.parse_sample_root(&attributes)?;
                        sampler_map[Clear] = Some(sampler);
                        active_sample_button = sampler_map[Clear].as_mut();
                        continue;
                    }
                }

                // Ends a tag with children
                Ok(Event::End(_)) => {}
                Ok(Event::Eof) => {
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    bail!("Error Parsing Profile: {}", e);
                }
            }
        }

        debug!("{:?}", mix_routing);
        debug!("{:?}", submix_tree);

        Ok(Self {
            root,
            browser,
            animation_tree,
            mix_routing,
            submix_tree,
            mixer,
            context,
            mute_chat,
            mute_buttons,
            faders,
            effects,
            scribbles,
            sampler_map,
            simple_elements,
            megaphone_effect,
            robot_effect,
            hardtune_effect,
            reverb_encoder,
            echo_encoder,
            pitch_encoder,
            gender_encoder,
        })
    }

    pub fn load_preset<R: Read>(&mut self, read: R) -> Result<()> {
        let buf_reader = BufReader::new(read);
        let mut reader = Reader::from_reader(buf_reader);

        // So, in principle here, all we need to do is loop over the tags, check on the
        // tag name, and load it directly into the relevant effect. This should force a
        // replace of the current effect, and bam, done.

        // Firstly, we need the current preset to overwrite.
        let current = self.context().selected_effects();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(ref e)) => {
                    let (name, attributes) = wrap_start_event(e)?;

                    match name.as_str() {
                        "reverbEncoder" => self
                            .reverb_encoder
                            .parse_reverb_preset(current, &attributes)?,
                        "echoEncoder" => {
                            self.echo_encoder.parse_echo_preset(current, &attributes)?
                        }
                        "pitchEncoder" => self
                            .pitch_encoder
                            .parse_pitch_preset(current, &attributes)?,
                        "genderEncoder" => self
                            .gender_encoder
                            .parse_gender_preset(current, &attributes)?,
                        "megaphoneEffect" => self
                            .megaphone_effect
                            .parse_megaphone_preset(current, &attributes)?,
                        "robotEffect" => {
                            self.robot_effect.parse_robot_preset(current, &attributes)?
                        }
                        "hardtuneEffect" => self
                            .hardtune_effect
                            .parse_hardtune_preset(current, &attributes)?,
                        _ => warn!("Unexpected Start Tag {}", name),
                    }
                }

                Ok(Event::Start(ref e)) => {
                    let (_name, attributes) = wrap_start_event(e)?;
                    let mut found = false;

                    // We can cheese this a little, there's only one tag in a preset that has
                    // children, and that's the top level element. So if this is going, we
                    // already know what to do.
                    for attribute in attributes {
                        if attribute.name == "name" {
                            found = true;
                            self.effects_mut(current).set_name(attribute.value)?;
                            break;
                        }
                    }
                    if !found {
                        bail!("Preset Name not found, cannot proceed.");
                    }
                }

                // Ends a tag with children
                Ok(Event::End(_)) => {}
                Ok(Event::Eof) => {
                    break;
                }

                Ok(_) => {}
                Err(_) => {}
            }
        }
        Ok(())
    }

    pub fn write<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let out_file = File::create(path)?;
        self.write_to(out_file)
    }

    pub fn write_to<W: Write>(&mut self, sink: W) -> Result<()> {
        let mut writer = Writer::new_with_indent(sink, u8::try_from('\t')?, 1);
        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;

        // For compatibility with the 'Release' version of the official app, we need to adjust
        // the config and reset channel monitoring back to the headphones (along with associated
        // routing), so we'll pull some data out, make some changes, then reload the settings once
        // writing is complete.
        let monitored_output = self.submix_tree.monitor_tree().monitored_output();
        let routing = self.mixer.mixer_table_mut();
        let headphone_routing = self.submix_tree.monitor_tree_mut().routing();
        let headphone_mix = self.submix_tree.monitor_tree_mut().headphone_mix();

        if monitored_output != OutputChannels::Headphones {
            for input in InputChannels::iter() {
                routing[input][OutputChannels::Headphones] = headphone_routing[input];
            }
            self.submix_tree
                .monitor_tree_mut()
                .set_monitored_output(OutputChannels::Headphones);
            self.submix_tree
                .monitor_tree_mut()
                .set_headphone_mix(Mix::A);
        }

        self.root.write_initial(&mut writer)?;
        self.browser.write_browser(&mut writer)?;
        self.animation_tree.write_animation(&mut writer)?;

        self.mix_routing.write_mix_tree(&mut writer)?;
        self.submix_tree.write_submixer(&mut writer)?;

        self.mixer.write_mixers(&mut writer)?;
        self.context.write_context(&mut writer)?;

        self.mute_chat.write_mute_chat(&mut writer)?;

        for (faders, mute_button) in self.mute_buttons.iter() {
            if let Some(mute_button) = mute_button {
                let name = format!("mute{}", (faders as u8) + 1);
                mute_button.write_button(name, &mut writer)?;
            }
        }

        for (faders, fader) in self.faders.iter() {
            if let Some(fader) = fader {
                let name = format!("FaderMeter{}", faders as u8);
                fader.write_fader(name, &mut writer)?;
            }
        }

        for (_key, value) in &self.effects {
            if let Some(value) = value {
                value.write_effects(&mut writer)?;
            }
        }

        for (_fader, scribble) in self.scribbles.iter() {
            if let Some(scribble) = scribble {
                scribble.write_scribble(&mut writer)?;
            }
        }

        self.megaphone_effect.write_megaphone(&mut writer)?;
        self.robot_effect.write_robot(&mut writer)?;
        self.hardtune_effect.write_hardtune(&mut writer)?;

        self.reverb_encoder.write_reverb(&mut writer)?;
        self.echo_encoder.write_echo(&mut writer)?;
        self.pitch_encoder.write_pitch(&mut writer)?;
        self.gender_encoder.write_gender(&mut writer)?;

        for (_key, value) in &self.sampler_map {
            if let Some(value) = value {
                value.write_sample(&mut writer)?;
            }
        }

        for simple_element in SimpleElements::iter() {
            self.simple_elements[simple_element]
                .as_ref()
                .unwrap()
                .write_simple(&mut writer)?;
        }

        // Finalise the XML..
        self.root.write_final(&mut writer)?;

        let routing = self.mixer.mixer_table_mut();
        // Everything's written, restore the original monitor settings..
        if monitored_output != OutputChannels::Headphones {
            for input in InputChannels::iter() {
                routing[input][OutputChannels::Headphones] = routing[input][monitored_output];
            }
            self.submix_tree
                .monitor_tree_mut()
                .set_monitored_output(monitored_output);
            self.submix_tree
                .monitor_tree_mut()
                .set_headphone_mix(headphone_mix);
        }

        Ok(())
    }

    pub fn write_preset<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let out_file = File::create(path)?;
        self.write_preset_to(&out_file)?;
        out_file.sync_all().context("Unable to Sync File")
    }

    pub fn write_preset_to<W: Write>(&self, sink: W) -> Result<()> {
        let mut writer = Writer::new_with_indent(sink, u8::try_from('\t')?, 1);
        writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;

        let current = self.context().selected_effects();
        let preset_writer = PresetWriter::new(String::from(self.effects(current).name()));
        preset_writer.write_initial(&mut writer)?;
        preset_writer.write_tag(
            &mut writer,
            "reverbEncoder",
            self.reverb_encoder.get_preset_attributes(current),
        )?;

        preset_writer.write_tag(
            &mut writer,
            "echoEncoder",
            self.echo_encoder.get_preset_attributes(current),
        )?;

        preset_writer.write_tag(
            &mut writer,
            "pitchEncoder",
            self.pitch_encoder.get_preset_attributes(current),
        )?;

        preset_writer.write_tag(
            &mut writer,
            "genderEncoder",
            self.gender_encoder.get_preset_attributes(current),
        )?;

        preset_writer.write_tag(
            &mut writer,
            "megaphoneEffect",
            self.megaphone_effect.get_preset_attributes(current),
        )?;

        preset_writer.write_tag(
            &mut writer,
            "robotEffect",
            self.robot_effect.get_preset_attributes(current),
        )?;

        preset_writer.write_tag(
            &mut writer,
            "hardtuneEffect",
            self.hardtune_effect.get_preset_attributes(current),
        )?;

        preset_writer.write_final(&mut writer)?;
        Ok(())
    }

    pub fn parse_preset(key: String) -> Result<Preset> {
        if let Some(id) = key
            .chars()
            .last()
            .map(|s| u8::from_str(&s.to_string()))
            .transpose()?
        {
            if let Some(preset) = Preset::iter().nth((id - 1) as usize) {
                return Ok(preset);
            }
        }
        Err(anyhow!("Unable to Parse Preset from Number"))
    }

    pub fn animation(&self) -> &AnimationTree {
        &self.animation_tree
    }

    pub fn animation_mut(&mut self) -> &mut AnimationTree {
        &mut self.animation_tree
    }

    pub fn mixer_mut(&mut self) -> &mut Mixers {
        &mut self.mixer
    }

    pub fn mixer(&self) -> &Mixers {
        &self.mixer
    }

    pub fn faders_mut(&mut self) -> &mut EnumMap<Faders, Option<Fader>> {
        &mut self.faders
    }

    pub fn fader_mut(&mut self, fader: Faders) -> &mut Fader {
        self.faders[fader].as_mut().unwrap()
    }

    pub fn fader(&self, fader: Faders) -> &Fader {
        self.faders[fader].as_ref().unwrap()
    }

    pub fn mute_buttons(&mut self) -> &mut EnumMap<Faders, Option<MuteButton>> {
        &mut self.mute_buttons
    }

    pub fn mute_button_mut(&mut self, fader: Faders) -> &mut MuteButton {
        self.mute_buttons[fader].as_mut().unwrap()
    }

    pub fn mute_button(&self, fader: Faders) -> &MuteButton {
        self.mute_buttons[fader].as_ref().unwrap()
    }

    pub fn scribbles_mut(&mut self) -> &mut EnumMap<Faders, Option<Scribble>> {
        &mut self.scribbles
    }

    pub fn scribble(&self, fader: Faders) -> &Scribble {
        self.scribbles[fader].as_ref().unwrap()
    }

    pub fn scribble_mut(&mut self, fader: Faders) -> &mut Scribble {
        self.scribbles[fader].as_mut().unwrap()
    }

    pub fn effects(&self, effect: Preset) -> &Effects {
        self.effects[effect].as_ref().unwrap()
    }

    pub fn effects_mut(&mut self, effect: Preset) -> &mut Effects {
        self.effects[effect].as_mut().unwrap()
    }

    pub fn mute_chat_mut(&mut self) -> &mut MuteChat {
        &mut self.mute_chat
    }

    pub fn mute_chat(&self) -> &MuteChat {
        &self.mute_chat
    }

    pub fn megaphone_effect(&self) -> &MegaphoneEffectBase {
        &self.megaphone_effect
    }

    pub fn megaphone_effect_mut(&mut self) -> &mut MegaphoneEffectBase {
        &mut self.megaphone_effect
    }

    pub fn robot_effect(&self) -> &RobotEffectBase {
        &self.robot_effect
    }

    pub fn robot_effect_mut(&mut self) -> &mut RobotEffectBase {
        &mut self.robot_effect
    }

    pub fn hardtune_effect(&self) -> &HardtuneEffectBase {
        &self.hardtune_effect
    }

    pub fn hardtune_effect_mut(&mut self) -> &mut HardtuneEffectBase {
        &mut self.hardtune_effect
    }

    pub fn sample_button(&self, button: SampleButtons) -> &SampleBase {
        self.sampler_map[button].as_ref().unwrap()
    }

    pub fn sample_button_mut(&mut self, button: SampleButtons) -> &mut SampleBase {
        self.sampler_map[button].as_mut().unwrap()
    }

    pub fn pitch_encoder(&self) -> &PitchEncoderBase {
        &self.pitch_encoder
    }

    pub fn pitch_encoder_mut(&mut self) -> &mut PitchEncoderBase {
        &mut self.pitch_encoder
    }

    pub fn echo_encoder(&self) -> &EchoEncoderBase {
        &self.echo_encoder
    }

    pub fn echo_encoder_mut(&mut self) -> &mut EchoEncoderBase {
        &mut self.echo_encoder
    }

    pub fn gender_encoder(&self) -> &GenderEncoderBase {
        &self.gender_encoder
    }

    pub fn gender_encoder_mut(&mut self) -> &mut GenderEncoderBase {
        &mut self.gender_encoder
    }

    pub fn reverb_encoder(&self) -> &ReverbEncoderBase {
        &self.reverb_encoder
    }

    pub fn reverb_encoder_mut(&mut self) -> &mut ReverbEncoderBase {
        &mut self.reverb_encoder
    }

    pub fn simple_element_mut(&mut self, name: SimpleElements) -> &mut SimpleElement {
        if self.simple_elements[name].is_some() {
            return self.simple_elements[name].as_mut().unwrap();
        }

        // If for whatever reason, this is missing, we'll use the global colour.
        return self.simple_elements[SimpleElements::GlobalColour]
            .as_mut()
            .unwrap();
    }

    pub fn simple_element(&self, name: SimpleElements) -> &SimpleElement {
        if self.simple_elements[name].is_some() {
            return self.simple_elements[name].as_ref().unwrap();
        }

        // If for whatever reason, this is missing, we'll use the global colour.
        return self.simple_elements[SimpleElements::GlobalColour]
            .as_ref()
            .unwrap();
    }

    pub fn context(&self) -> &Context {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    pub fn submixes(&self) -> &SubMixer {
        &self.submix_tree
    }
    pub fn submixes_mut(&mut self) -> &mut SubMixer {
        &mut self.submix_tree
    }

    pub fn mix_routing(&self) -> &MixRoutingTree {
        &self.mix_routing
    }
    pub fn mix_routing_mut(&mut self) -> &mut MixRoutingTree {
        &mut self.mix_routing
    }
}

/// This will wrap a 'Start' XML event into a name, and attribute Vec. We're using
/// our own Attribute Struct here to allow easy moving between XML libraries in future.
/// TODO: If we're doing this, we might as well make the attributes a HashMap
pub(crate) fn wrap_start_event(event: &BytesStart) -> Result<(String, Vec<Attribute>)> {
    let mut attributes = Vec::new();

    let name = String::from_utf8_lossy(event.local_name().as_ref()).parse()?;
    for attribute in event.attributes() {
        match attribute {
            Ok(a) => {
                attributes.push(Attribute {
                    name: String::from_utf8_lossy(a.key.local_name().as_ref()).parse()?,
                    value: String::from(a.unescape_value()?.as_ref()),
                });
            }
            Err(e) => {
                bail!("Error Processing Attribute: {}", e);
            }
        }
    }
    Ok((name, attributes))
}
