use std::{io::Write, path::PathBuf};

use anyhow::{Context, Error};
use fontc::{DisableFlags, Flags};
use skrifa::raw::{types::NameId, FontRef, TableProvider as _};
use tilvisan::Args as AutohintArgs;
use write_fonts::{
    from_obj::ToOwnedTable as _,
    tables::head::{Flags as HeadFlags, Head},
    FontBuilder,
};

fn main() -> Result<(), Error> {
    let input = std::env::args()
        .nth(1)
        .expect("Please provide input font path");
    // We need a way to tell fontc not to bother building a variable font, for
    // statics with more than one (incompatible) master in the sources.
    // We could try, and then recover if it fails, but that's ugly. Or we could
    // parse an existing gftools-builder config file, but that's heavy. And neither
    // approach actually gives us enough information - we need to know which instances
    // to build, since fontc needs the exact instance name. Of course we could then
    // start grubbing around in the source file for an instance list, but that's
    // *even heavier*, and this is *mvp*-fontc-builder.
    // So instead we determine both (a) the need to build static and (b) the specific
    // instances to build, by looking for a .instances file alongside the source.
    // If present, read one instance name per line, and build as statics.
    let path = PathBuf::from(&input);
    let mut instances_file = path.clone();
    instances_file.set_extension("instances");
    if instances_file.exists() {
        let instances_file_text =
            std::fs::read_to_string(&instances_file).context("reading instances file")?;
        let instances = instances_file_text
            .split("\n")
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        for instance in instances {
            build_a_font(&input, Some(instance));
        }
    } else {
        let compiled_font = compile_font(&input, None).expect("Failed to compile font");
        let autohinted_font = autohint(&compiled_font).expect("Failed to autohint font");
        let output_filename =
            determine_filename(&autohinted_font).expect("Failed to determine output filename");
        std::fs::write(&output_filename, &autohinted_font).expect("Failed to write output font");
        println!("{}", output_filename);
    }
    Ok(())
}

fn build_a_font(input_font: &str, instance: Option<String>) {
    let compiled_font = compile_font(input_font, instance).expect("Failed to compile font");
    let autohinted_font = autohint(&compiled_font).expect("Failed to autohint font");
    let output_filename =
        determine_filename(&autohinted_font).expect("Failed to determine output filename");
    std::fs::write(&output_filename, &autohinted_font).expect("Failed to write output font");
    println!("{}", output_filename);
}

fn fontc_options() -> fontc::Options {
    fontc::Options {
        flags: Flags::default() | Flags::DECOMPOSE_TRANSFORMED_COMPONENTS,
        flags_to_disable: DisableFlags::empty(),
        skip_features: false,
        compile_debg: false,
        output_file: None,
        debug_dir: None,
    }
}

fn compile_font(input_font: &str, instance: Option<String>) -> Result<Vec<u8>, Error> {
    // If a GlyphData.xml file is found, it should be loaded into the glyph_data vec.
    let path = PathBuf::from(input_font);
    let maybe_glyphdata = path.join("GlyphData.xml");
    let glyphdata = if maybe_glyphdata.exists() {
        vec![maybe_glyphdata]
    } else {
        vec![]
    };
    let input = fontc::Input::new(&path)
        .context("loading fontc input")?
        .create_source(&fontc::SourceOptions {
            instance,
            glyph_data: glyphdata,
        })
        .context("creating fontc source")?;
    let font = fontc::generate_font(input, fontc_options()).context("compiling font")?;
    Ok(font)
}

fn autohint(input_bytes: &[u8]) -> Result<Vec<u8>, Error> {
    // Save to temporary file
    let mut tmpfile = tempfile::NamedTempFile::new().context("creating temporary file")?;
    let font_filename = tmpfile.path().to_string_lossy().to_string();
    tmpfile
        .write_all(input_bytes)
        .context("writing to temporary file")?;
    let args = AutohintArgs {
        input: font_filename,
        ..Default::default()
    };
    // One day we should pick up on primary script and apply it to the args
    let font_data = tilvisan::autohint(&args).context("running autohint")?;

    // Fix integer PPEM
    let fontref = FontRef::new(&font_data).context("reading font data")?;
    let mut head: Head = fontref
        .head()
        .context("reading head table")?
        .to_owned_table();
    head.flags |= HeadFlags::FORCE_INTEGER_PPEM;
    let mut builder = FontBuilder::new();
    builder.add_table(&head).context("adding head table")?;
    builder.copy_missing_tables(fontref);
    Ok(builder.build())
}

// Google Fonts-specific filename determination logic.
fn determine_filename(font_bytes: &[u8]) -> Result<String, Error> {
    let font = skrifa::Font::new(font_bytes.to_vec(), 0).context("reading font binary")?;
    let familyname = font
        .names()
        .get(NameId::FAMILY_NAME)
        .and_then(|x| x.english_or_first())
        .map(|x| x.name().to_string())
        .unwrap_or("Unknown Font".to_string())
        .replace(" ", "");
    let subfamilyname = font
        .names()
        .get(NameId::TYPOGRAPHIC_SUBFAMILY_NAME)
        .or_else(|| font.names().get(NameId::SUBFAMILY_NAME))
        .and_then(|x| x.english_or_first())
        .map(|x| x.name().to_string())
        .unwrap_or("Regular".to_string())
        .replace(" ", "");

    let mut axes = font
        .axes()
        .iter()
        .filter(|x| !x.is_hidden())
        .map(|x| x.tag().to_string())
        .collect::<Vec<_>>();
    axes.sort();
    let suffix = if axes.is_empty() {
        format!("-{}", subfamilyname)
    } else {
        if subfamilyname.contains("Italic") {
            format!("-{}[{}]", subfamilyname, axes.join(","))
        } else {
            format!("[{}]", axes.join(","))
        }
    };
    Ok(format!("{}{}.ttf", familyname, suffix))
}
