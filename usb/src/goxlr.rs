use crate::channels::Channel;
use crate::commands::Command;
use crate::commands::SystemInfoCommand;
use crate::commands::SystemInfoCommand::SupportsDCPCategory;
use crate::dcp::DCPCategory;
use crate::error::ConnectError;
use crate::faders::Fader;
use byteorder::{ByteOrder, LittleEndian};
use log::info;
use rusb::{
    Device, DeviceDescriptor, DeviceHandle, Direction, GlobalContext, Language, Recipient,
    RequestType, UsbContext,
};
use std::thread::sleep;
use std::time::Duration;

pub struct GoXLR<T: UsbContext> {
    handle: DeviceHandle<T>,
    _device: Device<T>,
    _device_descriptor: DeviceDescriptor,
    timeout: Duration,
    _language: Language,
    command_count: u16,
}

const VID_GOXLR: u16 = 0x1220;
const PID_GOXLR_MINI: u16 = 0x8fe4;
const PID_GOXLR_FULL: u16 = 0x8fe0;

impl GoXLR<GlobalContext> {
    pub fn open() -> Result<Self, ConnectError> {
        let mut error = ConnectError::DeviceNotFound;
        for device in rusb::devices()?.iter() {
            if let Ok(descriptor) = device.device_descriptor() {
                if descriptor.vendor_id() == VID_GOXLR
                    && (descriptor.product_id() == PID_GOXLR_FULL
                        || descriptor.product_id() == PID_GOXLR_MINI)
                {
                    match device.open() {
                        Ok(handle) => return GoXLR::from_device(handle, descriptor),
                        Err(e) => error = e.into(),
                    }
                }
            }
        }

        Err(error)
    }
}

impl<T: UsbContext> GoXLR<T> {
    pub fn from_device(
        mut handle: DeviceHandle<T>,
        device_descriptor: DeviceDescriptor,
    ) -> Result<Self, ConnectError> {
        let device = handle.device();
        let timeout = Duration::from_secs(1);

        info!("Connected to possible GoXLR device at {:?}", device);

        let languages = handle.read_languages(timeout)?;
        let language = languages
            .get(0)
            .ok_or(ConnectError::DeviceNotGoXLR)?
            .to_owned();

        handle.set_active_configuration(1);
        handle.claim_interface(0);

        let mut goxlr = Self {
            handle,
            _device: device,
            _device_descriptor: device_descriptor,
            timeout,
            _language: language,
            command_count: 0,
        };

        goxlr.read_control(RequestType::Vendor, 0, 0, 0, 24)?; // ??

        goxlr.write_control(RequestType::Vendor, 1, 0, 0, &[])?;
        goxlr.read_control(RequestType::Vendor, 3, 0, 0, 1040)?; // ??

        Ok(goxlr)
    }

    pub fn read_control(
        &mut self,
        request_type: RequestType,
        request: u8,
        value: u16,
        index: u16,
        length: usize,
    ) -> Result<Vec<u8>, rusb::Error> {
        let mut buf = vec![0; length];
        let response_length = self.handle.read_control(
            rusb::request_type(Direction::In, request_type, Recipient::Interface),
            request,
            value,
            index,
            &mut buf,
            self.timeout,
        )?;
        buf.truncate(response_length);
        Ok(buf)
    }

    pub fn write_control(
        &mut self,
        request_type: RequestType,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
    ) -> Result<(), rusb::Error> {
        self.handle.write_control(
            rusb::request_type(Direction::Out, request_type, Recipient::Interface),
            request,
            value,
            index,
            data,
            self.timeout,
        )?;

        Ok(())
    }

    pub fn request_data(&mut self, command: Command, body: &[u8]) -> Result<Vec<u8>, rusb::Error> {
        self.command_count += 1;
        let command_index = self.command_count;
        let mut full_request = vec![0; 16];
        LittleEndian::write_u32(&mut full_request[0..4], command.command_id());
        LittleEndian::write_u16(&mut full_request[4..6], body.len() as u16);
        LittleEndian::write_u16(&mut full_request[6..8], command_index);
        full_request.extend(body);

        self.write_control(RequestType::Vendor, 2, 0, 0, &full_request)?;

        // TODO: A retry mechanism
        sleep(Duration::from_millis(10));
        self.await_interrupt(Duration::from_secs(2));

        let mut response_header = self.read_control(RequestType::Vendor, 3, 0, 0, 1040)?;
        let response = response_header.split_off(16);
        let response_length = LittleEndian::read_u16(&response_header[4..6]);
        let response_command_index = LittleEndian::read_u16(&response_header[6..8]);

        debug_assert!(response.len() == response_length as usize);
        debug_assert!(response_command_index == command_index);

        Ok(response)
    }

    pub fn supports_dcp_category(&mut self, category: DCPCategory) -> Result<bool, rusb::Error> {
        let mut out = [0; 2];
        LittleEndian::write_u16(&mut out, category.id());
        let result = self.request_data(Command::SystemInfo(SupportsDCPCategory), &out)?;
        Ok(LittleEndian::read_u16(&result) == 1)
    }

    pub fn get_system_info(&mut self) -> Result<(), rusb::Error> {
        let _result =
            self.request_data(Command::SystemInfo(SystemInfoCommand::FirmwareVersion), &[])?;
        // TODO: parse that?
        Ok(())
    }

    pub fn set_fader(&mut self, fader: Fader, channel: Channel) -> Result<(), rusb::Error> {
        // Channel ID, unknown, unknown, unknown
        self.request_data(Command::SetFader(fader), &[channel.id(), 0x00, 0x00, 0x00])?;
        Ok(())
    }

    pub fn set_volume(&mut self, channel: Channel, volume: u8) -> Result<(), rusb::Error> {
        self.request_data(Command::SetChannelVolume(channel), &[volume])?;
        Ok(())
    }

    pub fn set_channel_mute(&mut self, channel: Channel, muted: bool) -> Result<(), rusb::Error> {
        // If I ever discover this isn't a simple boolean, I'll change it.
        let state = if !muted { 0x00 } else { 0x01 } as u8;

        self.request_data(Command::SetChannelMute(channel), &[state])?;
        Ok(())
    }

    pub fn await_interrupt(&mut self, duration: Duration) -> Result<(), rusb::Error> {
        let mut buffer = [0u8; 6];
        self.handle.read_interrupt(0x81, &mut buffer, duration);
        Ok(())
    }
}
