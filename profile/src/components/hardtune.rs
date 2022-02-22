use std::collections::HashMap;
use std::fs::File;
use std::os::raw::c_float;
use std::str::FromStr;

use enum_map::EnumMap;
use strum::{Display, EnumIter, EnumProperty, EnumString, IntoEnumIterator};
use xml::attribute::OwnedAttribute;
use xml::writer::events::StartElementBuilder;
use xml::writer::XmlEvent as XmlWriterEvent;
use xml::EventWriter;

use crate::components::colours::ColourMap;
use crate::components::hardtune::HardtuneSource::All;
use crate::components::hardtune::HardtuneStyle::Normal;
use crate::components::megaphone::Preset;
use crate::components::megaphone::Preset::{Preset1, Preset2, Preset3, Preset4, Preset5, Preset6};

#[derive(thiserror::Error, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum ParseError {
    #[error("Expected int: {0}")]
    ExpectedInt(#[from] std::num::ParseIntError),

    #[error("Expected float: {0}")]
    ExpectedFloat(#[from] std::num::ParseFloatError),

    #[error("Expected enum: {0}")]
    ExpectedEnum(#[from] strum::ParseError),

    #[error("Invalid colours: {0}")]
    InvalidColours(#[from] crate::components::colours::ParseError),
}

/**
 * This is relatively static, main tag contains standard colour mapping, subtags contain various
 * presets, we'll use an EnumMap to define the 'presets' as they'll be useful for the other various
 * 'types' of presets (encoders and effects).
 */
#[derive(Debug)]
pub struct HardtuneEffectBase {
    colour_map: ColourMap,
    preset_map: EnumMap<Preset, HardtuneEffect>,
    source: HardtuneSource,
}

impl HardtuneEffectBase {
    pub fn new(element_name: String) -> Self {
        let colour_map = element_name;
        Self {
            colour_map: ColourMap::new(colour_map),
            preset_map: EnumMap::default(),
            source: Default::default(),
        }
    }

    pub fn parse_hardtune_root(&mut self, attributes: &[OwnedAttribute]) -> Result<(), ParseError> {
        for attr in attributes {
            // I honestly have no idea why this lives here :D
            if attr.name.local_name == "HARDTUNE_SOURCE" {
                self.source = HardtuneSource::from_str(&attr.value)?;
                continue;
            }

            if !self.colour_map.read_colours(attr)? {
                println!("[hardTuneEffect] Unparsed Attribute: {}", attr.name);
            }
        }

        Ok(())
    }

    pub fn parse_hardtune_preset(
        &mut self,
        id: u8,
        attributes: &[OwnedAttribute],
    ) -> Result<(), ParseError> {
        let mut preset = HardtuneEffect::new();
        for attr in attributes {
            if attr.name.local_name == "hardtuneEffectstate" {
                if attr.value == "1" {
                    preset.state = true;
                } else {
                    preset.state = false
                }
                continue;
            }
            if attr.name.local_name == "HARDTUNE_STYLE" {
                for style in HardtuneStyle::iter() {
                    if style.get_str("uiIndex").unwrap() == attr.value {
                        preset.style = style;
                        break;
                    }
                }
                continue;
            }

            if attr.name.local_name == "HARDTUNE_KEYSOURCE" {
                preset.keysource = attr.value.parse::<c_float>()? as u8;
                continue;
            }
            if attr.name.local_name == "HARDTUNE_AMOUNT" {
                preset.amount = attr.value.parse::<c_float>()? as u8;
                continue;
            }
            if attr.name.local_name == "HARDTUNE_WINDOW" {
                preset.window = attr.value.parse::<c_float>()? as u8;
                continue;
            }
            if attr.name.local_name == "HARDTUNE_RATE" {
                preset.rate = attr.value.parse::<c_float>()? as u8;
                continue;
            }
            if attr.name.local_name == "HARDTUNE_SCALE" {
                preset.scale = attr.value.parse::<c_float>()? as u8;
                continue;
            }
            if attr.name.local_name == "HARDTUNE_PITCH_AMT" {
                preset.pitch_amt = attr.value.parse::<c_float>()? as u8;
                continue;
            }
            if attr.name.local_name == "HARDTUNE_SOURCE" {
                preset.source = Option::Some(HardtuneSource::from_str(&attr.value)?);
                continue;
            }

            println!(
                "[HardtuneEffect] Unparsed Child Attribute: {}",
                &attr.name.local_name
            );
        }

        // Ok, we should be able to store this now..
        if id == 1 {
            self.preset_map[Preset1] = preset;
        } else if id == 2 {
            self.preset_map[Preset2] = preset;
        } else if id == 3 {
            self.preset_map[Preset3] = preset;
        } else if id == 4 {
            self.preset_map[Preset4] = preset;
        } else if id == 5 {
            self.preset_map[Preset5] = preset;
        } else if id == 6 {
            self.preset_map[Preset6] = preset;
        }

        Ok(())
    }

    pub fn write_hardtune(
        &self,
        writer: &mut EventWriter<&mut File>,
    ) -> Result<(), xml::writer::Error> {
        let mut element: StartElementBuilder = XmlWriterEvent::start_element("hardtuneEffect");

        let mut attributes: HashMap<String, String> = HashMap::default();
        attributes.insert("HARDTUNE_SOURCE".to_string(), self.source.to_string());
        self.colour_map.write_colours(&mut attributes);

        // Write out the attributes etc for this element, but don't close it yet..
        for (key, value) in &attributes {
            element = element.attr(key.as_str(), value.as_str());
        }

        writer.write(element)?;

        // Because all of these are seemingly 'guaranteed' to exist, we can straight dump..
        for (key, value) in &self.preset_map {
            let mut sub_attributes: HashMap<String, String> = HashMap::default();

            let tag_name = format!("hardtuneEffect{}", key.get_str("tagSuffix").unwrap());
            let mut sub_element: StartElementBuilder =
                XmlWriterEvent::start_element(tag_name.as_str());

            sub_attributes.insert(
                "hardtuneEffectstate".to_string(),
                if value.state {
                    "1".to_string()
                } else {
                    "0".to_string()
                },
            );
            sub_attributes.insert(
                "HARDTUNE_STYLE".to_string(),
                value.style.get_str("uiIndex").unwrap().to_string(),
            );
            sub_attributes.insert(
                "HARDTUNE_KEYSOURCE".to_string(),
                format!("{}", value.keysource),
            );
            sub_attributes.insert("HARDTUNE_AMOUNT".to_string(), format!("{}", value.amount));
            sub_attributes.insert("HARDTUNE_WINDOW".to_string(), format!("{}", value.window));
            sub_attributes.insert("HARDTUNE_RATE".to_string(), format!("{}", value.rate));
            sub_attributes.insert("HARDTUNE_SCALE".to_string(), format!("{}", value.scale));
            sub_attributes.insert(
                "HARDTUNE_PITCH_AMT".to_string(),
                format!("{}", value.pitch_amt),
            );

            if let Some(source) = &value.source {
                sub_attributes.insert("HARDTUNE_SOURCE".to_string(), source.to_string());
            }

            for (key, value) in &sub_attributes {
                sub_element = sub_element.attr(key.as_str(), value.as_str());
            }

            writer.write(sub_element)?;
            writer.write(XmlWriterEvent::end_element())?;
        }

        // Finally, close the 'main' tag.
        writer.write(XmlWriterEvent::end_element())?;
        Ok(())
    }

    pub fn colour_map(&self) -> &ColourMap {
        &self.colour_map
    }
}

#[derive(Debug, Default)]
struct HardtuneEffect {
    // State here determines if the hardtune is on or off when this preset is loaded.
    state: bool,

    style: HardtuneStyle,
    keysource: u8,
    amount: u8,
    window: u8,
    rate: u8,
    scale: u8,
    pitch_amt: u8,
    source: Option<HardtuneSource>,
}

impl HardtuneEffect {
    pub fn new() -> Self {
        Self {
            state: false,
            style: Default::default(),
            keysource: 0,
            amount: 0,
            window: 0,
            rate: 0,
            scale: 0,
            pitch_amt: 0,
            source: None,
        }
    }
}

#[derive(Debug, EnumIter, EnumProperty)]
enum HardtuneStyle {
    #[strum(props(uiIndex = "0"))]
    Normal,

    #[strum(props(uiIndex = "1"))]
    Medium,

    #[strum(props(uiIndex = "2"))]
    Hard,
}

impl Default for HardtuneStyle {
    fn default() -> Self {
        Normal
    }
}

#[derive(Debug, Display, EnumString)]
enum HardtuneSource {
    #[strum(to_string = "ALL")]
    All,

    #[strum(to_string = "MUSIC")]
    Music,

    #[strum(to_string = "GAME")]
    Game,

    #[strum(to_string = "LINEIN")]
    LineIn,
}

impl Default for HardtuneSource {
    fn default() -> Self {
        All
    }
}
