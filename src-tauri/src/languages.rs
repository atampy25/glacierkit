use crate::game_detection::GameVersion;

/// Get the language map (and whether to use a symmetric cipher) for the given game version and iteration.
pub fn get_language_map(version: GameVersion, iteration: u8) -> Option<(bool, Vec<String>)> {
	match version {
		GameVersion::H1 => (vec!["xx", "en", "fr", "it", "de", "es", "ru", "mx", "br", "pl", "cn", "jp"].len()
			> iteration.into())
		.then(|| {
			(
				iteration != 0,
				vec!["xx", "en", "fr", "it", "de", "es", "ru", "mx", "br", "pl", "cn", "jp"]
					.into_iter()
					.rev()
					.skip(iteration.into())
					.rev()
					.map(|x| x.into())
					.collect::<Vec<_>>()
			)
		}),
		GameVersion::H2 => (iteration == 0).then_some((
			false,
			vec![
				"xx", "en", "fr", "it", "de", "es", "ru", "mx", "br", "pl", "cn", "jp", "tc",
			]
			.into_iter()
			.map(|x| x.into())
			.collect()
		)),
		GameVersion::H3 => match iteration {
			0 => Some((
				false,
				vec!["xx", "en", "fr", "it", "de", "es", "ru", "cn", "tc", "jp"]
					.into_iter()
					.map(|x| x.into())
					.collect()
			)),
			1 => Some((
				false,
				vec!["xx", "en", "fr", "it", "de", "es"]
					.into_iter()
					.map(|x| x.into())
					.collect()
			)),
			_ => None
		}
	}
}
