use super::*;
use crate::avm2::{Class, Namespace, QName};
use crate::backend::audio::RegisteredSound;
use crate::character::Character;
use crate::library::Library;
use crate::string::AvmString;
use crate::tag_utils::{SwfMovie, SwfMovieData};
use gc_arena::{Arena, Collect, Gc, Rootable};
use ruffle_wstr::WStr;
use slotmap::Key;

fn pcm(data: &[u8]) -> swf::Sound<'_> {
    swf::Sound {
        id: 1,
        format: swf::SoundFormat {
            compression: AudioCompression::Uncompressed,
            sample_rate: 44100,
            is_stereo: false,
            is_16_bit: true,
        },
        num_samples: (data.len() / 2) as u32,
        data,
    }
}

fn loop_settings() -> swf::SoundInfo {
    swf::SoundInfo {
        event: swf::SoundEvent::Event,
        in_sample: Some(2),
        out_sample: Some(9),
        num_loops: 3,
        envelope: None,
    }
}

#[test]
fn aola_mixer_movie_reload_slope() {
    const ROUNDS: usize = 48;
    const SOUNDS: usize = 16;
    const BYTES: usize = 16384;
    let payload = vec![23; BYTES];
    for maintenance in [false, true] {
        let mut mixer = AudioMixer::new(2, 44100);
        mixer.track_definitions = maintenance;
        let raw = mixer.register_sound(&pcm(&payload)).unwrap();
        // A backend consumer that never registered a lifetime stays valid.
        assert!(mixer.retain_sound_definition(raw).is_none());

        let mut arena = Arena::<Rootable![(Library<'_>, Option<SwfMovie<'_>>)]>::new(|_| {
            (Library::empty(), None)
        });
        for round in 1..=ROUNDS {
            arena.mutate_root(|mc, (library, root)| {
                let movie = SwfMovie::new(mc, SwfMovieData::empty(10, None));
                let mut symbols = library.library_for_movie_mut(movie, mc);
                for id in 1..=SOUNDS {
                    let handle = mixer.register_sound(&pcm(&payload)).unwrap();
                    let sound = RegisteredSound {
                        handle,
                        _owner: mixer.track_sound_definition(handle).map(|owner| Gc::new(mc, owner)),
                    };
                    assert!(symbols.register_character(id as u16, Character::Sound(sound)));
                }
                *root = Some(movie);
            });
            arena.finish_cycle();
            mixer.maintain_sound_definitions();
            assert_eq!(mixer.sound_definition_stats().registered, 1 + if maintenance { SOUNDS } else { round * SOUNDS });
            arena.mutate_root(|_, (_, root)| *root = None);
            arena.finish_cycle();
            arena.mutate_root(|mc, (library, _)| library.after_collection(mc));
            mixer.maintain_sound_definitions();
            let stats = mixer.sound_definition_stats();
            let expected = 1 + if maintenance { 0 } else { round * SOUNDS };
            assert_eq!((stats.registered, stats.tracked, stats.active_instances), (expected, 0, 0));
            assert_eq!(stats.encoded_bytes, (expected * BYTES) as u64);
            assert_eq!(stats.retired, if maintenance { (round * SOUNDS) as u64 } else { 0 });
            if round % 12 == 0 {
                println!("maintenance={maintenance} round={round} {stats:?}");
            }
        }
        assert!(mixer.start_sound(raw, &loop_settings()).is_ok());
    }
}

#[derive(Collect)]
#[collect(no_drop)]
struct SymbolRoots<'gc> {
    library: Library<'gc>,
    class: Option<Class<'gc>>,
    importer: Option<SwfMovie<'gc>>,
}

#[test]
fn aola_mixer_export_import_and_late_class_lookup_after_gc() {
    let mut mixer = AudioMixer::new(2, 44100);
    mixer.track_definitions = true;
    let handle = mixer.register_sound(&pcm(&[42; 128])).unwrap();
    let mut arena = Arena::<Rootable![SymbolRoots<'_>]>::new(|mc| {
        let mut library = Library::empty();
        let movie = SwfMovie::new(mc, SwfMovieData::empty(10, None));
        let importer = SwfMovie::new(mc, SwfMovieData::empty(10, None));
        let name = AvmString::new_utf8(mc, "SharedSound");
        let sound = RegisteredSound {
            handle,
            _owner: Some(Gc::new(mc, mixer.track_sound_definition(handle).unwrap())),
        };
        let exported = {
            let mut symbols = library.library_for_movie_mut(movie, mc);
            symbols.register_character(1, Character::Sound(sound));
            symbols.register_export(1, name);
            // Same lookup as AVM1 Sound.attachSound and the import-assets path.
            symbols.character_by_export_name(&name).unwrap().1
        };
        let mut imports = library.library_for_movie_mut(importer, mc);
        imports.register_import(name, 99);
        imports.register_character(99, exported);
        drop(imports);
        let class = Class::custom_new(QName::new(Namespace::any(), name), None, None, Box::new([]), mc);
        library.avm2_class_registry_mut().set_class_symbol(class, movie, 1, mc);
        SymbolRoots { library, class: Some(class), importer: Some(importer) }
    });
    arena.finish_cycle();
    mixer.maintain_sound_definitions();
    arena.mutate_root(|_, root| {
        // Resolve after GC through exactly the class-symbol lookup used by flash.media.Sound.init.
        let (movie, id) = root.library.avm2_class_registry().class_symbol(root.class.unwrap()).unwrap();
        let symbols = root.library.library_for_movie(movie).unwrap();
        let Character::Sound(sound) = symbols.character_by_id(id).unwrap() else { panic!() };
        let channel = mixer.start_sound(sound.handle, &loop_settings()).unwrap();
        mixer.stop_sound(channel);
        root.class = None;
    });
    arena.finish_cycle();
    mixer.maintain_sound_definitions();
    arena.mutate_root(|_, root| {
        let imports = root.library.library_for_movie(root.importer.unwrap()).unwrap();
        let id = imports.character_id_by_import_name(WStr::from_units(b"SharedSound")).unwrap();
        assert_eq!(imports.get_sound(id), Some(handle));
        let channel = mixer.start_sound(handle, &loop_settings()).unwrap();
        mixer.stop_sound(channel);
        root.importer = None;
    });
    arena.finish_cycle();
    mixer.maintain_sound_definitions();
    assert_eq!(mixer.sound_definition_stats().retired, 1);
}

#[test]
fn aola_mixer_preserves_playback_completion_and_handle_generations() {
    let payload: Vec<_> = (0..32i16).flat_map(|i| (i * 600).to_le_bytes()).collect();
    let mut reference = None;
    for maintenance in [false, true] {
        let mut mixer = AudioMixer::new(2, 44100);
        mixer.track_definitions = maintenance;
        let handle = mixer.register_sound(&pcm(&payload)).unwrap();
        let owner = mixer.track_sound_definition(handle);
        let channel = mixer.start_sound(handle, &loop_settings()).unwrap();
        drop(owner);
        mixer.maintain_sound_definitions();
        assert_eq!(mixer.sound_definition_stats().active_instances, 1);
        assert!(mixer.get_sound_position(channel).is_some());
        let completion = mixer.retain_sound_definition(handle);
        let mut output = [0i16; 128];
        mixer.proxy().mix(&mut output);
        assert!(output.iter().any(|sample| *sample != 0));
        if let Some(expected) = reference {
            assert_eq!(output, expected);
        }
        reference = Some(output);
        assert!(mixer.get_sound_position(channel).is_none());
        mixer.maintain_sound_definitions();
        assert!(mixer.get_sound_duration(handle).is_some());
        drop(completion);
        mixer.maintain_sound_definitions();
        if maintenance {
            assert!(mixer.get_sound_duration(handle).is_none());
            let replacement = mixer.register_sound(&pcm(&payload)).unwrap();
            assert_eq!(handle.data().as_ffi() as u32, replacement.data().as_ffi() as u32);
            assert_ne!(handle, replacement);
            assert!(matches!(mixer.start_sound(handle, &loop_settings()), Err(DecodeError::UnknownSound)));
            assert!(mixer.track_sound_definition(handle).is_none());
            assert!(mixer.start_sound(replacement, &loop_settings()).is_ok());
        } else {
            assert!(mixer.get_sound_duration(handle).is_some());
        }
    }
}

#[test]
fn aola_mixer_embedded_mp3_keeps_only_payload_and_preserves_seek() {
    let mut backing = vec![0; 1 << 20];
    backing[512..518].copy_from_slice(&[35, 0, 9, 8, 7, 6]);
    let mut mixer = AudioMixer::new(2, 44100);
    let mut sound = pcm(&backing[512..518]);
    sound.format.compression = AudioCompression::Mp3;
    let handle = mixer.register_sound(&sound).unwrap();
    sound.data = &backing[..1];
    assert!(matches!(mixer.register_sound(&sound), Err(RegisterError::ShortMp3)));
    assert_ne!(mixer.sounds[handle].data.as_ptr(), backing[514..].as_ptr());
    drop(backing);
    assert_eq!(&*mixer.sounds[handle].data, &[9, 8, 7, 6]);
    assert_eq!(mixer.sounds[handle].skip_sample_frames, 35);
    assert_eq!(mixer.sound_definition_stats().encoded_bytes, 4);
}
