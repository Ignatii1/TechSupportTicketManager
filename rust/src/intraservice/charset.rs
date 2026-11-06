//! Тело ответа — строкой, как `ReaderOf` C#-клиента: по BOM, если он есть (главнее заголовка), иначе по кодировке из
//! заголовка, если она здесь знакома (`TryEncoding`), иначе UTF-8; битые байты — знаком замены (U+FFFD), как у .NET.
//! ponytail: знакомы UTF-8, UTF-16/32, windows-1251, KOI8-R и ISO-8859-1 — .NET знает все кодовые страницы, но страницы
//! IIS и прокси по-русски приходят в 1251; незнакомая — UTF-8.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Utf16 { big_endian: bool },
    Utf32 { big_endian: bool },
    Table(&'static [u16; 128]),
    Latin1,
}

/// Кодировка по имени из `Content-Type` (`charset=…`, в кавычках — тоже; регистр — любой). None — незнакомая.
fn encoding(charset: Option<&str>) -> Option<Encoding> {
    let name = charset?.trim_matches('"').to_ascii_lowercase();
    Some(match name.as_str() {
        "utf-8" | "utf8" => Encoding::Utf8,
        "utf-16" | "utf-16le" | "unicode" | "ucs-2" => Encoding::Utf16 { big_endian: false },
        "utf-16be" | "unicodefffe" => Encoding::Utf16 { big_endian: true },
        "utf-32" | "utf-32le" => Encoding::Utf32 { big_endian: false },
        "utf-32be" => Encoding::Utf32 { big_endian: true },
        "windows-1251" | "cp1251" | "x-cp1251" => Encoding::Table(&CP1251),
        "koi8-r" | "koi8r" | "cskoi8r" => Encoding::Table(&KOI8_R),
        "iso-8859-1" | "iso_8859-1" | "latin1" | "l1" => Encoding::Latin1,
        _ => return None,
    })
}

/// Знакома ли кодировка из заголовка (`TryEncoding`): нет, пусто или незнакомая (utf-7 .NET знает, но не читает) — нет.
pub fn known(charset: Option<&str>) -> bool {
    encoding(charset).is_some()
}

/// Тело ответа строкой: BOM, затем `charset` из заголовка, затем UTF-8.
pub fn decode(bytes: &[u8], charset: Option<&str>) -> String {
    let (encoding, body) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE, 0x00, 0x00]) {
        (Encoding::Utf32 { big_endian: false }, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (Encoding::Utf16 { big_endian: false }, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (Encoding::Utf16 { big_endian: true }, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0x00, 0x00, 0xFE, 0xFF]) {
        (Encoding::Utf32 { big_endian: true }, rest)
    } else {
        (encoding(charset).unwrap_or(Encoding::Utf8), bytes)
    };
    match encoding {
        Encoding::Utf8 => String::from_utf8_lossy(body).into_owned(),
        Encoding::Utf16 { big_endian } => {
            let units =
                body.chunks_exact(2).map(|p| if big_endian { u16::from_be_bytes([p[0], p[1]]) } else { u16::from_le_bytes([p[0], p[1]]) });
            let mut text: String = char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
            if body.len() % 2 == 1 {
                text.push(char::REPLACEMENT_CHARACTER);
            }
            text
        }
        Encoding::Utf32 { big_endian } => {
            let mut text: String = body
                .chunks_exact(4)
                .map(|p| {
                    let n = if big_endian {
                        u32::from_be_bytes([p[0], p[1], p[2], p[3]])
                    } else {
                        u32::from_le_bytes([p[0], p[1], p[2], p[3]])
                    };
                    char::from_u32(n).unwrap_or(char::REPLACEMENT_CHARACTER)
                })
                .collect();
            if body.len() % 4 != 0 {
                text.push(char::REPLACEMENT_CHARACTER);
            }
            text
        }
        Encoding::Table(table) => body
            .iter()
            .map(|&b| {
                if b < 0x80 {
                    char::from(b)
                } else {
                    char::from_u32(u32::from(table[usize::from(b - 0x80)])).unwrap_or(char::REPLACEMENT_CHARACTER)
                }
            })
            .collect(),
        Encoding::Latin1 => body.iter().map(|&b| char::from(b)).collect(),
    }
}

/// windows-1251, байты 0x80–0xFF (кодек Python; 0x98 у Windows — U+0098).
const CP1251: [u16; 128] = [
    0x0402, 0x0403, 0x201A, 0x0453, 0x201E, 0x2026, 0x2020, 0x2021, 0x20AC, 0x2030, 0x0409, 0x2039, 0x040A, 0x040C, 0x040B, 0x040F, 0x0452,
    0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x0098, 0x2122, 0x0459, 0x203A, 0x045A, 0x045C, 0x045B, 0x045F, 0x00A0, 0x040E,
    0x045E, 0x0408, 0x00A4, 0x0490, 0x00A6, 0x00A7, 0x0401, 0x00A9, 0x0404, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x0407, 0x00B0, 0x00B1, 0x0406,
    0x0456, 0x0491, 0x00B5, 0x00B6, 0x00B7, 0x0451, 0x2116, 0x0454, 0x00BB, 0x0458, 0x0405, 0x0455, 0x0457, 0x0410, 0x0411, 0x0412, 0x0413,
    0x0414, 0x0415, 0x0416, 0x0417, 0x0418, 0x0419, 0x041A, 0x041B, 0x041C, 0x041D, 0x041E, 0x041F, 0x0420, 0x0421, 0x0422, 0x0423, 0x0424,
    0x0425, 0x0426, 0x0427, 0x0428, 0x0429, 0x042A, 0x042B, 0x042C, 0x042D, 0x042E, 0x042F, 0x0430, 0x0431, 0x0432, 0x0433, 0x0434, 0x0435,
    0x0436, 0x0437, 0x0438, 0x0439, 0x043A, 0x043B, 0x043C, 0x043D, 0x043E, 0x043F, 0x0440, 0x0441, 0x0442, 0x0443, 0x0444, 0x0445, 0x0446,
    0x0447, 0x0448, 0x0449, 0x044A, 0x044B, 0x044C, 0x044D, 0x044E, 0x044F,
];

/// KOI8-R, байты 0x80–0xFF (кодек Python).
const KOI8_R: [u16; 128] = [
    0x2500, 0x2502, 0x250C, 0x2510, 0x2514, 0x2518, 0x251C, 0x2524, 0x252C, 0x2534, 0x253C, 0x2580, 0x2584, 0x2588, 0x258C, 0x2590, 0x2591,
    0x2592, 0x2593, 0x2320, 0x25A0, 0x2219, 0x221A, 0x2248, 0x2264, 0x2265, 0x00A0, 0x2321, 0x00B0, 0x00B2, 0x00B7, 0x00F7, 0x2550, 0x2551,
    0x2552, 0x0451, 0x2553, 0x2554, 0x2555, 0x2556, 0x2557, 0x2558, 0x2559, 0x255A, 0x255B, 0x255C, 0x255D, 0x255E, 0x255F, 0x2560, 0x2561,
    0x0401, 0x2562, 0x2563, 0x2564, 0x2565, 0x2566, 0x2567, 0x2568, 0x2569, 0x256A, 0x256B, 0x256C, 0x00A9, 0x044E, 0x0430, 0x0431, 0x0446,
    0x0434, 0x0435, 0x0444, 0x0433, 0x0445, 0x0438, 0x0439, 0x043A, 0x043B, 0x043C, 0x043D, 0x043E, 0x043F, 0x044F, 0x0440, 0x0441, 0x0442,
    0x0443, 0x0436, 0x0432, 0x044C, 0x044B, 0x0437, 0x0448, 0x044D, 0x0449, 0x0447, 0x044A, 0x042E, 0x0410, 0x0411, 0x0426, 0x0414, 0x0415,
    0x0424, 0x0413, 0x0425, 0x0418, 0x0419, 0x041A, 0x041B, 0x041C, 0x041D, 0x041E, 0x041F, 0x042F, 0x0420, 0x0421, 0x0422, 0x0423, 0x0416,
    0x0412, 0x042C, 0x042B, 0x0417, 0x0428, 0x042D, 0x0429, 0x0427, 0x042A,
];
