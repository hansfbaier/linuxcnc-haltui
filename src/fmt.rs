//! Mini C-printf-style formatter for the halshow `ffmts` / `ifmts`
//! settings (e.g. "%5.2f", "%08x", "%-10s").

#[derive(Clone, Debug)]
pub enum FmtArg {
    Int(i64),
    Float(f64),
}

/// Apply a single printf-style conversion. Returns the rendered string.
pub fn apply(fmt: &str, arg: &FmtArg) -> String {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i >= chars.len() {
            break;
        }
        if chars[i] == '%' {
            out.push('%');
            i += 1;
            continue;
        }
        // flags
        let mut minus = false;
        let mut plus = false;
        let mut space = false;
        let mut zero = false;
        let mut hash = false;
        while i < chars.len() {
            match chars[i] {
                '-' => minus = true,
                '+' => plus = true,
                ' ' => space = true,
                '0' => zero = true,
                '#' => hash = true,
                _ => break,
            }
            i += 1;
        }
        // width
        let mut width: Option<usize> = None;
        while i < chars.len() && chars[i].is_ascii_digit() {
            width = Some(width.unwrap_or(0) * 10 + chars[i].to_digit(10).unwrap() as usize);
            i += 1;
        }
        // precision
        let mut prec: Option<usize> = None;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            prec = Some(0);
            while i < chars.len() && chars[i].is_ascii_digit() {
                prec = Some(prec.unwrap_or(0) * 10 + chars[i].to_digit(10).unwrap() as usize);
                i += 1;
            }
        }
        if i >= chars.len() {
            break;
        }
        let conv = chars[i];
        i += 1;
        let (core, numeric) = match arg {
            FmtArg::Int(v) => (int_conv(conv, *v, prec, plus, space, hash), true),
            FmtArg::Float(v) => (float_conv(conv, *v, prec, plus, space), true),
        };
        out.push_str(&pad(&core, width, minus, zero && numeric));
    }
    out
}

fn int_conv(
    conv: char,
    v: i64,
    prec: Option<usize>,
    plus: bool,
    space: bool,
    hash: bool,
) -> String {
    let mut core = match conv {
        'd' | 'i' => format!("{}", v),
        'u' => format!("{}", v as u64),
        'x' => {
            let h = if hash { "0x" } else { "" };
            format!("{}{:x}", h, v as u64)
        }
        'X' => {
            let h = if hash { "0X" } else { "" };
            format!("{}{:X}", h, v as u64)
        }
        'o' => {
            let h = if hash { "0" } else { "" };
            format!("{}{:o}", h, v as u64)
        }
        'c' => (v as u8 as char).to_string(),
        _ => format!("{}", v),
    };
    // sign flags
    if v >= 0 && matches!(conv, 'd' | 'i') {
        if plus {
            core.insert(0, '+');
        } else if space {
            core.insert(0, ' ');
        }
    }
    // zero-pad numeric to precision (digits after sign)
    if let Some(p) = prec {
        let sign_len = core
            .chars()
            .take_while(|c| *c == '-' || *c == '+' || *c == ' ')
            .count();
        let digits = core.chars().count() - sign_len;
        if p > digits {
            let zeros = "0".repeat(p - digits);
            core.insert_str(sign_len, &zeros);
        }
    }
    core
}

fn float_conv(conv: char, v: f64, prec: Option<usize>, plus: bool, space: bool) -> String {
    let mut core = match conv {
        'f' | 'F' => format!("{:.*}", prec.unwrap_or(6), v),
        'e' => format!("{:.*e}", prec.unwrap_or(6), v),
        'E' => format!("{:.*E}", prec.unwrap_or(6), v),
        'g' | 'G' => match prec {
            Some(p) => format!("{:.*}", p, v),
            None => format!("{}", v),
        },
        _ => format!("{}", v),
    };
    if v >= 0.0 && !v.is_nan() {
        if plus {
            core.insert(0, '+');
        } else if space {
            core.insert(0, ' ');
        }
    }
    core
}

fn pad(core: &str, width: Option<usize>, left: bool, zero: bool) -> String {
    let w = match width {
        Some(w) if w > core.chars().count() => w,
        _ => return core.to_string(),
    };
    let padn = w - core.chars().count();
    if left {
        format!("{}{}", core, " ".repeat(padn))
    } else if zero {
        // insert zeros between sign and digits
        let sign_len = core
            .chars()
            .take_while(|c| *c == '-' || *c == '+' || *c == ' ')
            .count();
        let (sign, digits) = core.split_at(sign_len);
        format!("{}{}{}", sign, "0".repeat(padn), digits)
    } else {
        format!("{}{}", " ".repeat(padn), core)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_width_precision() {
        assert_eq!(apply("%5.2f", &FmtArg::Float(3.14159)), " 3.14");
        assert_eq!(apply("%.1f", &FmtArg::Float(2.5)), "2.5");
        assert_eq!(apply("%08.1f", &FmtArg::Float(-2.5)), "-00002.5");
        assert_eq!(apply("%+d", &FmtArg::Int(42)), "+42");
        assert_eq!(apply("%08x", &FmtArg::Int(255)), "000000ff");
        assert_eq!(apply("%-6d|", &FmtArg::Int(42)), "42    |");
        assert_eq!(apply("%e", &FmtArg::Float(1234.5)), "1.234500e3");
        assert_eq!(apply("100%%", &FmtArg::Int(0)), "100%");
    }
}
