use glacier_commons::game::GlacierGame;

/// Get the language map (and whether to use a symmetric cipher) for the given game version and iteration.
pub fn get_language_map(version: GlacierGame, iteration: u8) -> Option<(bool, Option<Vec<String>>)> {
	match version {
		GlacierGame::H1 => (vec!["xx", "en", "fr", "it", "de", "es", "ru", "mx", "br", "pl", "cn", "jp"].len()
			> iteration.into())
		.then(|| {
			(
				iteration != 0,
				Some(
					vec!["xx", "en", "fr", "it", "de", "es", "ru", "mx", "br", "pl", "cn", "jp"]
						.into_iter()
						.rev()
						.skip(iteration.into())
						.rev()
						.map(|x| x.into())
						.collect::<Vec<_>>()
				)
			)
		}),
		GlacierGame::H2 => (iteration == 0).then_some((
			false,
			Some(
				vec![
					"xx", "en", "fr", "it", "de", "es", "ru", "mx", "br", "pl", "cn", "jp", "tc",
				]
				.into_iter()
				.map(|x| x.into())
				.collect()
			)
		)),
		GlacierGame::H3 => match iteration {
			0 => Some((false, None)),
			1 => Some((
				false,
				Some(
					vec!["xx", "en", "fr", "it", "de", "es"]
						.into_iter()
						.map(|x| x.into())
						.collect()
				)
			)),
			_ => None
		},
		GlacierGame::FL => match iteration {
			// TODO
			0 => Some((false, None)),
			1 => Some((
				false,
				Some(
					vec!["xx", "en", "fr", "it", "de", "es"]
						.into_iter()
						.map(|x| x.into())
						.collect()
				)
			)),
			_ => None
		}
	}
}
