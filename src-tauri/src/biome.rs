use std::str::FromStr;

use anyhow::{Context, Result};
use biome_formatter::{IndentStyle, IndentWidth, LineEnding, LineWidth};
use biome_json_formatter::context::{JsonFormatOptions, TrailingCommas};
use biome_json_parser::JsonParserOptions;
use fn_error_context::context;
use tryvial::try_fn;

#[try_fn]
#[context("Couldn't format JSON")]
pub fn format_json(data: &str) -> Result<String> {
	biome_json_formatter::format_node(
		JsonFormatOptions::new()
			.with_indent_style(IndentStyle::Tab)
			.with_indent_width(IndentWidth::from(4))
			.with_line_ending(LineEnding::Lf)
			.with_line_width(LineWidth::from_str("75").unwrap())
			.with_trailing_commas(TrailingCommas::None),
		&biome_json_parser::parse_json(
			data,
			JsonParserOptions {
				allow_comments: true,
				allow_trailing_commas: true
			}
		)
		.syntax()
	)
	.context("Couldn't format with Biome")?
	.print()
	.context("Couldn't print formatted JSON")?
	.into_code()
}
