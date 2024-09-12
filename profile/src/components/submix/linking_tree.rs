use crate::components::mixer::InputChannels;
use crate::profile::Attribute;
use anyhow::{bail, Result};
use enum_map::EnumMap;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Writer;
use std::collections::HashMap;
use std::io::Write;
use strum::{EnumProperty, IntoEnumIterator};

#[derive(Debug)]
pub struct LinkingTree {
    linked_list: EnumMap<InputChannels, bool>,
    linked_ratio: EnumMap<InputChannels, f64>,
}

impl Default for LinkingTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkingTree {
    pub fn new() -> Self {
        Self {
            linked_list: LinkingTree::get_default_linked_list(),
            linked_ratio: LinkingTree::get_default_linked_ratio(),
        }
    }

    fn get_default_linked_list() -> EnumMap<InputChannels, bool> {
        let mut linked: EnumMap<InputChannels, bool> = Default::default();
        for channel in InputChannels::iter() {
            linked[channel] = true;
        }
        linked
    }

    fn get_default_linked_ratio() -> EnumMap<InputChannels, f64> {
        let mut linked: EnumMap<InputChannels, f64> = Default::default();
        for channel in InputChannels::iter() {
            linked[channel] = 1.0_f64;
        }
        linked
    }

    pub fn parse_links(&mut self, attributes: &Vec<Attribute>) -> Result<()> {
        for attr in attributes {
            if attr.name.ends_with("Linked") {
                let mut found = false;

                // Get the String key..
                let channel = attr.name.as_str();
                let channel = &channel[0..channel.len() - 6];

                let value: bool = matches!(attr.value.as_str(), "1");

                // Find the channel from the Prefix..
                for chan_enum in InputChannels::iter() {
                    if chan_enum.get_str("Name").unwrap() == channel {
                        // Set the value..
                        self.linked_list[chan_enum] = value;
                        found = true;
                    }
                }

                if !found {
                    println!("Unable to find Channel: {channel}");
                }
                continue;
            }

            if attr.name.ends_with("Ratio") {
                let mut found = false;

                // Get the String key..
                let channel = attr.name.as_str();
                let channel = &channel[0..channel.len() - 5];

                let value: f64 = attr.value.parse().unwrap_or(1.);

                // Find the channel from the Prefix..
                for chan_enum in InputChannels::iter() {
                    if chan_enum.get_str("Name").unwrap() == channel {
                        // Set the value..
                        self.linked_ratio[chan_enum] = value;
                        found = true;
                    }
                }

                if !found {
                    println!("Unable to find Channel: {channel}");
                }
                continue;
            }
        }

        Ok(())
    }

    pub fn write_linking_tree<W: Write>(&self, writer: &mut Writer<W>) -> Result<()> {
        let mut elem = BytesStart::new("linkingTree");

        // This one's actually incredibly straight forward :)
        let mut attributes: HashMap<String, String> = HashMap::default();
        for input in InputChannels::iter() {
            let key = format!("{}Linked", input.get_str("Name").unwrap());
            let value = format!("{}", self.linked_list[input] as u8);

            attributes.insert(key, value);
        }

        for input in InputChannels::iter() {
            let key = format!("{}Ratio", input.get_str("Name").unwrap());
            let value = format!("{}", self.linked_ratio[input]);

            attributes.insert(key, value);
        }

        // Set the attributes into the XML object..
        for (key, value) in &attributes {
            elem.push_attribute((key.as_str(), value.as_str()));
        }

        writer.write_event(Event::Empty(elem))?;
        Ok(())
    }

    pub fn is_linked(&self, channel: InputChannels) -> bool {
        self.linked_list[channel]
    }
    pub fn get_ratio(&self, channel: InputChannels) -> f64 {
        if self.linked_ratio[channel] == f64::INFINITY {
            // Caused by a divide by 0, return 255 / 1
            255.
        } else if self.linked_ratio[channel] == 0. {
            // Caused by a 0 / value..
            1. / 255.
        } else {
            self.linked_ratio[channel]
        }
    }

    pub fn set_link_enabled(&mut self, channel: InputChannels, linked: bool) -> Result<()> {
        self.linked_list[channel] = linked;
        Ok(())
    }

    pub fn set_link_ratio(&mut self, channel: InputChannels, ratio: f64) -> Result<()> {
        if ratio == 0. || ratio == f64::INFINITY {
            bail!("Invalid Ratio");
        }
        self.linked_ratio[channel] = ratio;
        Ok(())
    }
}
