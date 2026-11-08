//! Hash-pinned OCR model packs. Sources: oar-ocr GitHub releases (same bytes as
//! the oar-ocr `REGISTRY` on ModelScope). Bumping: see `playbooks/natives-and-models.md`.

/// One downloadable model file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// A language pack: everything the engine needs for one script family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pack {
    pub id: &'static str,
    pub description: &'static str,
    /// `ocr.language` codes served by this pack.
    pub languages: &'static [&'static str],
    pub det: ModelFile,
    pub rec: ModelFile,
    pub dict: ModelFile,
    /// PP-LCNet page orientation (0/90/180/270), on by default.
    pub doc_orientation: Option<ModelFile>,
    /// PP-LCNet text-line orientation (0/180), on by default.
    pub textline_orientation: Option<ModelFile>,
}

impl Pack {
    pub fn files(&self) -> Vec<ModelFile> {
        let mut v = vec![self.det, self.rec, self.dict];
        v.extend(self.doc_orientation);
        v.extend(self.textline_orientation);
        v
    }

    pub fn total_size(&self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }
}

const GH_V070: &str = "https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/";
const GH_V030: &str = "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/";

pub const PP_OCRV6_SMALL_DET: ModelFile = ModelFile {
    name: "pp-ocrv6_small_det.onnx",
    url: "https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/pp-ocrv6_small_det.onnx",
    sha256: "d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e",
    size: 9_880_512,
};
pub const PP_OCRV6_SMALL_REC: ModelFile = ModelFile {
    name: "pp-ocrv6_small_rec.onnx",
    url: "https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/pp-ocrv6_small_rec.onnx",
    sha256: "5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634",
    size: 21_159_378,
};
pub const PP_OCRV6_DICT: ModelFile = ModelFile {
    name: "ppocrv6_dict.txt",
    url: "https://github.com/GreatV/oar-ocr/releases/download/v0.7.0/ppocrv6_dict.txt",
    sha256: "b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d",
    size: 74_947,
};
pub const PP_LCNET_DOC_ORI: ModelFile = ModelFile {
    name: "pp-lcnet_x1_0_doc_ori.onnx",
    url: "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-lcnet_x1_0_doc_ori.onnx",
    sha256: "bbcd6c2b43ab15d2e605455aef2cd280ab87b570824522d24a69e9298875a1ac",
    size: 6_787_248,
};
pub const PP_LCNET_TEXTLINE_ORI: ModelFile = ModelFile {
    name: "pp-lcnet_x0_25_textline_ori.onnx",
    url: "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-lcnet_x0_25_textline_ori.onnx",
    sha256: "fb402220f39b183d64a68cd48d4bd53267a21354b6fc39370c2a83fbdad85b10",
    size: 1_018_629,
};

/// Latin script (incl. English) on PP-OCRv6 small. The only pack shipped so far;
/// other scripts (PP-OCRv5 mobile rec: korean, th, el, te, ta, eslav, arabic,
/// cyrillic, devanagari; server rec for ch/japan) are a followup.
pub const LATIN: Pack = Pack {
    id: "latin",
    description: "Latin-script languages incl. English (PP-OCRv6 small)",
    languages: &[
        "en", "af", "az", "bs", "ca", "cs", "cy", "da", "de", "es", "et", "eu", "fi", "fr", "ga",
        "gl", "hr", "hu", "id", "is", "it", "ku", "la", "lb", "lt", "lv", "mi", "ms", "mt", "nl",
        "no", "oc", "pi", "pl", "pt", "qu", "rm", "ro", "rs_latin", "sk", "sl", "sq", "sv", "sw",
        "tl", "tr", "uz", "vi",
    ],
    det: PP_OCRV6_SMALL_DET,
    rec: PP_OCRV6_SMALL_REC,
    dict: PP_OCRV6_DICT,
    doc_orientation: Some(PP_LCNET_DOC_ORI),
    textline_orientation: Some(PP_LCNET_TEXTLINE_ORI),
};

pub const PACKS: &[Pack] = &[LATIN];

/// Release base URLs (documentation / bump helper).
pub const RELEASE_BASES: [&str; 2] = [GH_V070, GH_V030];

pub fn pack(id: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|p| p.id == id)
}

/// `ocr.language` value for "every installed pack" (the default).
pub const AUTO_LANGUAGE: &str = "auto";

/// Is `lang` the automatic setting (case-insensitive)?
pub fn is_auto(lang: &str) -> bool {
    lang.trim().eq_ignore_ascii_case(AUTO_LANGUAGE)
}

/// Pack for an `ocr.language` value (case-insensitive): a language code
/// or a pack id (`latin`). `auto` maps to the default pack (the first one);
/// [`crate::ocr::pack_for_setting`] picks among installed packs instead.
pub fn pack_for_language(lang: &str) -> Option<&'static Pack> {
    if is_auto(lang) {
        return PACKS.first();
    }
    let lang = lang.trim().to_ascii_lowercase();
    PACKS
        .iter()
        .find(|p| p.id == lang || p.languages.contains(&lang.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_well_formed() {
        for p in PACKS {
            for f in p.files() {
                assert_eq!(f.sha256.len(), 64, "{}", f.name);
                assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(f.url.ends_with(f.name));
                assert!(RELEASE_BASES.iter().any(|b| f.url.starts_with(b)));
                assert!(f.size > 0);
            }
        }
        assert_eq!(LATIN.total_size(), 38_920_714);
    }

    #[test]
    fn language_mapping() {
        assert_eq!(pack_for_language("en").map(|p| p.id), Some("latin"));
        assert_eq!(pack_for_language(" DE ").map(|p| p.id), Some("latin"));
        assert_eq!(pack_for_language("ja"), None);
        assert_eq!(pack_for_language(" Auto ").map(|p| p.id), Some("latin"));
        assert_eq!(pack_for_language("Latin").map(|p| p.id), Some("latin"));
    }
}
