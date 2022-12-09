/*
This is simply a struct that manages and returns a list of files in various directories.

I considered sending this data on-demand, however things like the UI may poll incredibly
frequently, and given the infrequency of changes holding a 1 second cache is useful.

This has been created as a separate mod primarily because profile.rs is big enough, and
secondly because it's managing different types of files
 */

use std::collections::BTreeMap;
use std::fs;
use std::fs::{create_dir_all, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use futures::channel::mpsc::{channel, Receiver};
use futures::executor::block_on;
use futures::{SinkExt, StreamExt};
use log::{debug, info, warn};

use glob::glob;
use goxlr_ipc::PathTypes;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::Sender;

use crate::{SettingsHandle, Shutdown, DISTRIBUTABLE_ROOT};

#[derive(Debug)]
pub struct FilePaths {
    profiles: PathBuf,
    mic_profiles: PathBuf,
    presets: PathBuf,
    icons: PathBuf,
    samples: PathBuf,
}

#[derive(Debug)]
pub struct FileManager {
    paths: FilePaths,
}

impl FileManager {
    pub fn new(settings: &SettingsHandle) -> Self {
        Self {
            paths: get_file_paths_from_settings(settings),
        }
    }

    pub fn get_profiles(&mut self) -> Vec<String> {
        let path = self.paths.profiles.clone();
        let extension = ["goxlr"].to_vec();

        let distrib_path = Path::new(DISTRIBUTABLE_ROOT).join("profiles/");
        self.get_files_from_paths(vec![distrib_path, path], extension, false)
    }

    pub fn get_mic_profiles(&mut self) -> Vec<String> {
        let path = self.paths.mic_profiles.clone();
        let extension = ["goxlrMicProfile"].to_vec();

        self.get_files_from_paths(vec![path], extension, false)
    }

    pub fn get_presets(&mut self) -> Vec<String> {
        let path = self.paths.presets.clone();
        let distrib_path = Path::new(DISTRIBUTABLE_ROOT).join("presets/");
        let extension = ["preset"].to_vec();

        self.get_files_from_paths(vec![path, distrib_path], extension, false)
    }

    pub fn get_samples(&mut self) -> BTreeMap<String, String> {
        let base_path = self.paths.samples.clone();
        let extensions = ["wav", "mp3"].to_vec();

        self.get_recursive_file_list(base_path, extensions)
    }

    pub fn get_icons(&mut self) -> Vec<String> {
        let path = self.paths.icons.clone();
        let extension = ["gif", "jpg", "png"].to_vec();

        self.get_files_from_paths(vec![path], extension, true)
    }

    fn get_recursive_file_list(
        &self,
        path: PathBuf,
        extensions: Vec<&str>,
    ) -> BTreeMap<String, String> {
        let mut paths: Vec<PathBuf> = Vec::new();

        for extension in extensions {
            let format = format!("{}/**/*.{}", path.to_string_lossy(), extension);
            let files = glob(format.as_str());
            if let Ok(files) = files {
                files.for_each(|f| paths.push(f.unwrap()));
            }
        }

        let mut map: BTreeMap<String, String> = BTreeMap::new();
        // Ok, we need to split stuff up..
        for file_path in paths {
            map.insert(
                file_path.to_string_lossy()[path.to_string_lossy().len() + 1..].to_string(),
                file_path.file_name().unwrap().to_string_lossy().to_string(),
            );
        }
        map
    }

    fn get_files_from_paths(
        &self,
        paths: Vec<PathBuf>,
        extensions: Vec<&str>,
        with_extension: bool,
    ) -> Vec<String> {
        let mut result = Vec::new();

        for path in paths {
            result.extend(self.get_files_from_drive(path, extensions.clone(), with_extension));
        }

        result.sort_by_key(|a| a.to_lowercase());
        result
    }

    fn get_files_from_drive(
        &self,
        path: PathBuf,
        extensions: Vec<&str>,
        with_extension: bool,
    ) -> Vec<String> {
        if let Err(error) = create_path(&path) {
            warn!(
                "Unable to create path: {}: {}",
                &path.to_string_lossy(),
                error
            );
        }

        if let Ok(list) = path.read_dir() {
            return list
                .filter_map(|entry| {
                    entry
                        .ok()
                        // Make sure this has an extension..
                        .filter(|e| e.path().extension().is_some())
                        // Is it the extension we're looking for?
                        .filter(|e| {
                            let path = e.path();
                            let os_ext = path.extension().unwrap();
                            for extension in extensions.clone() {
                                if extension == os_ext {
                                    return true;
                                }
                            }
                            false
                        })
                        // Get the File Name..
                        .and_then(|e| {
                            return if with_extension {
                                e.path()
                                    .file_name()
                                    .and_then(|n| n.to_str().map(String::from))
                            } else {
                                e.path().file_stem().and_then(
                                    // Convert it to a String..
                                    |n| n.to_str().map(String::from),
                                )
                            };
                        })
                    // Collect the result.
                })
                .collect::<Vec<String>>();
        }

        if !path.starts_with(Path::new(DISTRIBUTABLE_ROOT)) {
            debug!(
                "Path not found, or unable to read: {:?}",
                path.to_string_lossy()
            );
        }

        Vec::new()
    }
}

//pub async fn run_notification_service(&self, sender: Sender<PathTypes>) -> Result<()> {
pub async fn run_notification_service(
    paths: FilePaths,
    sender: Sender<PathTypes>,
    mut shutdown_signal: Shutdown,
) {
    let watcher = create_watcher();
    if let Err(error) = watcher {
        warn!("Error Creating the File Watcher, aborting: {:?}", error);
        return;
    }

    // Create the worker..
    let (mut watcher, mut rx) = watcher.unwrap();

    // Add the Paths to the Watcher..
    if let Err(error) = watcher.watch(&paths.profiles, RecursiveMode::NonRecursive) {
        warn!("Unable to Monitor Profiles Path: {:?}", error);
    };
    if let Err(error) = watcher.watch(&paths.mic_profiles, RecursiveMode::NonRecursive) {
        warn!("Unable to Monitor the Microphone Profile Path {:?}", error);
    };
    if let Err(error) = watcher.watch(&paths.presets, RecursiveMode::NonRecursive) {
        warn!("Unable to Monitor the Presets Path: {:?}", error)
    };
    if let Err(error) = watcher.watch(&paths.icons, RecursiveMode::NonRecursive) {
        warn!("Unable to monitor the Icons Path: {:?}", error);
    }

    if let Err(error) = watcher.watch(&paths.samples, RecursiveMode::Recursive) {
        warn!("Unable to Monitor the Samples Path: {:?}", error);
    }

    let mut last_send = Instant::now();

    // Wait for any changes..
    loop {
        tokio::select! {
            () = shutdown_signal.recv() => {
                debug!("Shutdown Signal Received.");
                break;
            },
            result = rx.next() => {
                if let Some(result) = result {
                    match result {
                        Ok(event) => {
                            debug!("{:?}", event);
                            match event.kind {
                                // Triggered on the Creation of a file / folder..
                                EventKind::Create(CreateKind::File) |
                                EventKind::Create(CreateKind::Folder) |
                                EventKind::Create(CreateKind::Any) |

                                // Triggered on the Removal of a File / Folder
                                EventKind::Remove(RemoveKind::File) |
                                EventKind::Remove(RemoveKind::Folder) |
                                EventKind::Remove(RemoveKind::Any) |

                                // Triggered on Rename / Move of a file
                                EventKind::Modify(ModifyKind::Name(RenameMode::From)) |
                                EventKind::Modify(ModifyKind::Name(RenameMode::To)) |
                                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {

                                    // Things like file creation, moving and deletion can send multiple
                                    // valid events, we don't need to spam all of them up, so use a small buffer.
                                    if last_send + Duration::from_millis(50) < Instant::now() {
                                        debug!("Useful Event Received! {:?}", event);
                                        last_send = Instant::now();

                                        let path = &event.paths[0];
                                        if path.starts_with(&paths.profiles) {
                                            let _ = sender.send(PathTypes::Profiles).await;
                                            continue;
                                        }

                                        if path.starts_with(&paths.mic_profiles) {
                                            let result = sender.send(PathTypes::MicProfiles).await;
                                            debug!("{:?}", result);
                                            continue;
                                        }

                                        if path.starts_with(&paths.icons) {
                                            let _ = sender.send(PathTypes::Icons).await;
                                            continue;
                                        }

                                        if path.starts_with(&paths.presets) {
                                            let _ = sender.send(PathTypes::Presets).await;
                                            continue;
                                        }

                                        if path.starts_with(&paths.samples) {
                                            let _ = sender.send(PathTypes::Samples).await;
                                            continue;
                                        }
                                    }
                                },

                                _ => {
                                    // Do nothing, not our kind of event!
                                }
                            }
                        },
                        Err(error) => {
                            warn!("Error Reading File Event: {:?}", error);
                        }
                    }
                }
            }
        }
    }
}

fn create_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    let watcher = RecommendedWatcher::new(
        move |res| {
            block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

pub fn get_file_paths_from_settings(settings: &SettingsHandle) -> FilePaths {
    FilePaths {
        profiles: block_on(settings.get_profile_directory()),
        mic_profiles: block_on(settings.get_mic_profile_directory()),
        presets: block_on(settings.get_presets_directory()),
        icons: block_on(settings.get_icons_directory()),
        samples: block_on(settings.get_samples_directory()),
    }
}

pub fn find_file_in_path(path: PathBuf, file: PathBuf) -> Option<PathBuf> {
    let format = format!("{}/**/{}", path.to_string_lossy(), file.to_string_lossy());
    let files = glob(format.as_str());
    if let Ok(files) = files {
        if let Some(file) = files.into_iter().next() {
            return Some(file.unwrap());
        }
    }

    None
}

pub fn create_path(path: &Path) -> Result<()> {
    if path.starts_with(Path::new(DISTRIBUTABLE_ROOT)) {
        return Ok(());
    }
    if !path.exists() {
        // Attempt to create the profile directory..
        if let Err(e) = create_dir_all(path) {
            return Err(e).context(format!("Could not create path {}", &path.to_string_lossy()))?;
        } else {
            info!("Created Path: {}", path.to_string_lossy());
        }
    }
    Ok(())
}

pub fn can_create_new_file(path: PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_path(parent)?;
    }

    if path.exists() {
        return Err(anyhow!("File already exists."));
    }

    // Attempt to create a file in the path, throw an error if fails..
    File::create(&path)?;

    // Remove the file again.
    fs::remove_file(&path)?;

    Ok(())
}
