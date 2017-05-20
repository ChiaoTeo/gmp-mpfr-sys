// Copyright © 2017 University of Malta

// This program is free software: you can redistribute it and/or
// modify it under the terms of the GNU Lesser General Public License
// as published by the Free Software Foundation, either version 3 of
// the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful, but
// WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU
// General Public License for more details.
//
// You should have received a copy of the GNU Lesser General Public
// License and a copy of the GNU General Public License along with
// this program. If not, see <http://www.gnu.org/licenses/>.

// Notes:
//
// 1. Configure GMP with --enable-fat so that built file is portable.
//
// 2. Configure GMP, MPFR and MPC with: --disable-shared --with-pic
//
// 3. Add symlinks to work around relative path issues in MPFR and MPC.
//    In MPFR: ln -s ../gmp-build
//    In MPC: ln -s ../mpfr-src ../mpfr-build ../gmp-build .
//
// 4. Use relative paths for configure otherwise msys/mingw might be
//    confused with drives and such.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const GMP_DIR: &'static str = "gmp-6.1.2-slim";
const MPFR_DIR: &'static str = "mpfr-3.1.5-slim";
const MPC_DIR: &'static str = "mpc-1.0.3-slim";

fn main() {
    let src_dir = PathBuf::from(cargo_env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(cargo_env("OUT_DIR"));
    let jobs = cargo_env("NUM_JOBS");
    let profile = cargo_env("PROFILE");
    let check = profile == OsString::from("release");
    let feature_mpc = cargo_has_env("CARGO_FEATURE_MPC");
    let feature_mpfr = feature_mpc || cargo_has_env("CARGO_FEATURE_MPFR");

    let lib_dir = out_dir.join("lib");
    let build_dir = out_dir.join("build");
    let gmp_lib = lib_dir.join("libgmp.a");
    let gmp_header = lib_dir.join("gmp.h");
    let mpfr_lib = lib_dir.join("libmpfr.a");
    let mpfr_header = lib_dir.join("mpfr.h");
    let mpc_lib = lib_dir.join("libmpc.a");
    let mpc_header = lib_dir.join("mpc.h");
    let compile_mpc = feature_mpc &&
                      (!mpc_lib.is_file() || !mpc_header.is_file());
    let compile_mpfr = compile_mpc ||
                       (feature_mpfr &&
                        (!mpfr_lib.is_file() || !mpfr_header.is_file()));
    let compile_gmp = compile_mpfr || !gmp_lib.is_file() ||
                      !gmp_header.is_file();
    if compile_gmp {
        create_dir(&lib_dir);
        remove_dir(&build_dir);
        create_dir(&build_dir);
        symlink(&build_dir,
                &dir_relative(&build_dir, &src_dir.join(GMP_DIR)),
                Some(&OsString::from("gmp-src")));
        build_gmp(&build_dir, &jobs, check, &gmp_lib, &gmp_header);
    }
    if compile_mpfr {
        symlink(&build_dir,
                &dir_relative(&build_dir, &src_dir.join(MPFR_DIR)),
                Some(&OsString::from("mpfr-src")));
        build_mpfr(&build_dir, &jobs, check, &mpfr_lib, &mpfr_header);
    }
    if compile_mpc {
        symlink(&build_dir,
                &dir_relative(&build_dir, &src_dir.join(MPC_DIR)),
                Some(&OsString::from("mpc-src")));
        build_mpc(&build_dir, &jobs, check, &mpc_lib, &mpc_header);
    }
    if compile_gmp {
        remove_dir(&build_dir);
    }
    process_gmp_header(&gmp_header, &out_dir.join("gmp_h.rs"));
    write_cargo_info(&lib_dir, feature_mpfr, feature_mpc);
}

fn build_gmp(top_build_dir: &Path,
             jobs: &OsStr,
             check: bool,
             lib: &Path,
             header: &Path) {
    let build_dir = top_build_dir.join("gmp-build");
    create_dir(&build_dir);
    println!("$ cd \"{}\"", build_dir.display());
    let conf = "../gmp-src/configure --enable-fat --disable-shared --with-pic";
    configure(&build_dir, &OsString::from(conf));
    make_and_check(&build_dir, &jobs, check);
    let build_lib = build_dir.join(".libs").join("libgmp.a");
    copy_file(&build_lib, &lib);
    let build_header = build_dir.join("gmp.h");
    copy_file(&build_header, &header);
}

fn process_gmp_header(header: &Path, out_file: &Path) {
    let mut limb_bits = None;
    let mut nail_bits = None;
    let mut long_long_limb = None;
    let mut cc = None;
    let mut cflags = None;
    let mut reader = open(&header);
    let mut buf = String::new();
    while read_line(&mut reader, &mut buf, &header) > 0 {
        if buf.contains("#undef _LONG_LONG_LIMB") {
            long_long_limb = Some(false);
        }
        if buf.contains("#define _LONG_LONG_LIMB 1") {
            long_long_limb = Some(true);
        }
        let s = "#define GMP_LIMB_BITS";
        if let Some(start) = buf.find(s) {
            limb_bits = buf[(start + s.len())..].trim().parse::<i32>().ok();
        }
        let s = "#define GMP_NAIL_BITS";
        if let Some(start) = buf.find(s) {
            nail_bits = buf[(start + s.len())..].trim().parse::<i32>().ok();
        }
        let s = "#define __GMP_CC";
        if let Some(start) = buf.find(s) {
            cc = Some(buf[(start + s.len())..]
                          .trim()
                          .trim_matches('"')
                          .to_string());
        }
        let s = "#define __GMP_CFLAGS";
        if let Some(start) = buf.find(s) {
            cflags = Some(buf[(start + s.len())..]
                              .trim()
                              .trim_matches('"')
                              .to_string());
        }
        buf.clear();
    }
    drop(reader);
    let limb_bits = limb_bits
        .expect("Cannot determine GMP_LIMB_BITS from gmp.h");
    let nail_bits = nail_bits
        .expect("Cannot determine GMP_NAIL_BITS from gmp.h");
    let long_long_limb =
        long_long_limb.expect("Cannot determine _LONG_LONG_LIMB from gmp.h");
    let long_long_limb = if long_long_limb {
        "::std::os::raw::c_ulonglong"
    } else {
        "::std::os::raw::c_ulong"
    };
    let cc = cc.expect("Cannot determine __GMP_CC from gmp.h");
    let cflags = cflags.expect("Cannot determine __GMP_CFLAGS from gmp.h");
    let content = format!(concat!("const GMP_LIMB_BITS: c_int = {};\n",
                                  "const GMP_NAIL_BITS: c_int = {};\n",
                                  "type GMP_LIMB_T = {};\n",
                                  "const GMP_CC: *const c_char =\n",
                                  "    b\"{}\\0\" as *const _ as _;\n",
                                  "const GMP_CFLAGS: *const c_char =\n",
                                  "    b\"{}\\0\" as *const _ as _;\n"),
                          limb_bits,
                          nail_bits,
                          long_long_limb,
                          cc,
                          cflags);

    let mut rs = create(out_file);
    write(&mut rs, &content, out_file);
    flush(&mut rs, out_file);
}

fn build_mpfr(top_build_dir: &Path,
              jobs: &OsStr,
              check: bool,
              lib: &Path,
              header: &Path) {
    let build_dir = top_build_dir.join("mpfr-build");
    create_dir(&build_dir);
    println!("$ cd {}", build_dir.display());
    symlink(&build_dir, &OsString::from("../gmp-build"), None);
    let conf = "../mpfr-src/configure --enable-thread-safe --disable-shared \
                --with-gmp-build=../gmp-build --with-pic";
    configure(&build_dir, &OsString::from(conf));
    make_and_check(&build_dir, &jobs, check);
    let build_lib = build_dir.join("src").join(".libs").join("libmpfr.a");
    copy_file(&build_lib, &lib);
    let src_header = top_build_dir.join("mpfr-src").join("src").join("mpfr.h");
    copy_file(&src_header, &header);
}

fn build_mpc(top_build_dir: &Path,
             jobs: &OsStr,
             check: bool,
             lib: &Path,
             header: &Path) {
    let build_dir = top_build_dir.join("mpc-build");
    create_dir(&build_dir);
    println!("$ cd {}", build_dir.display());
    symlink(&build_dir, &OsString::from("../mpfr-src"), None);
    symlink(&build_dir, &OsString::from("../mpfr-build"), None);
    symlink(&build_dir, &OsString::from("../gmp-build"), None);
    let conf = "../mpc-src/configure --disable-shared \
                --with-mpfr-include=../mpfr-src/src \
                --with-mpfr-lib=../mpfr-build/src/.libs \
                --with-gmp-include=../gmp-build \
                --with-gmp-lib=../gmp-build/.libs --with-pic";
    configure(&build_dir, &OsString::from(conf));
    make_and_check(&build_dir, &jobs, check);
    let build_lib = build_dir.join("src").join(".libs").join("libmpc.a");
    copy_file(&build_lib, &lib);
    let src_header = top_build_dir.join("mpc-src").join("src").join("mpc.h");
    copy_file(&src_header, &header);
}

fn write_cargo_info(lib_dir: &Path, feature_mpfr: bool, feature_mpc: bool) {
    let lib_search = lib_dir
        .to_str()
        .unwrap_or_else(|| {
            panic!("Path contains unsupported characters, can only make {}",
                   lib_dir.display())
        });
    println!("cargo:rustc-link-search=native={}", lib_search);
    println!("cargo:rustc-link-lib=static=gmp");
    if feature_mpfr {
        println!("cargo:rustc-link-lib=static=mpfr");
    }
    if feature_mpc {
        println!("cargo:rustc-link-lib=static=mpc");
    }
    check_mingw(feature_mpfr, feature_mpc);
}

fn cargo_env(name: &str) -> OsString {
    env::var_os(name).unwrap_or_else(|| {
        panic!("environment variable not found: {}, please use cargo", name)
    })
}

fn cargo_has_env(name: &str) -> bool {
    env::var_os(name).is_some()
}

fn check_mingw(feature_mpfr: bool, _feature_mpc: bool) {
    // extra libraries needed only for mpfr because of thread-local storage
    if !feature_mpfr {
        return;
    }

    for check in &["HOST", "TARGET"] {
        if !cargo_env(check)
                .into_string()
                .map(|s| s.ends_with("-windows-gnu"))
                .unwrap_or(false) {
            return;
        }
    }

    // link to gcc_eh
    println!("cargo:rustc-link-lib=static=gcc_eh");

    // also link to pthread, but only if rustc version >= 1.18
    if rustc_later_eq(1, 18) {
        println!("cargo:rustc-link-lib=static=pthread");
    }
}

fn rustc_later_eq(major: i32, minor: i32) -> bool {
    let rustc = cargo_env("RUSTC");
    let output = Command::new(rustc)
        .arg("--version")
        .output()
        .expect("unable to run rustc --version");
    let version = String::from_utf8(output.stdout)
        .expect("unrecognized rustc version");
    if !version.starts_with("rustc ") {
        panic!("unrecognized rustc version");
    }
    let remain = &version[6..];
    let dot = remain.find('.').expect("unrecognized rustc version");
    let ver_major = remain[0..dot]
        .parse::<i32>()
        .expect("unrecognized rustc version");
    if ver_major < major {
        return false;
    } else if ver_major > major {
        return true;
    }
    let remain = &remain[dot + 1..];
    let dot = remain.find('.').expect("unrecognized rustc version");
    let ver_minor = remain[0..dot]
        .parse::<i32>()
        .expect("unrecognized rustc version");
    ver_minor >= minor
}

fn remove_dir(dir: &Path) {
    if !dir.exists() {
        return;
    }
    assert!(dir.is_dir(), "Not a directory: {}", dir.display());
    fs::remove_dir_all(dir).unwrap_or_else(|_| {
        panic!("Unable to remove directory: {}", dir.display())
    });
}

fn create_dir(dir: &Path) {
    fs::create_dir_all(dir).unwrap_or_else(|_| {
        panic!("Unable to create directory: {}", dir.display())
    });
}

fn dir_relative(dir: &Path, rel_to: &Path) -> OsString {
    let (mut diri, mut reli) = (dir.components(), rel_to.components());
    let (mut dirc, mut relc) = (diri.next(), reli.next());
    let mut some_common = false;
    while let (Some(d), Some(r)) = (dirc, relc) {
        if d != r {
            break;
        }
        some_common = true;
        dirc = diri.next();
        relc = reli.next();
    }
    assert!(some_common,
            "cannot access {} from {} using relative paths",
            rel_to.display(),
            dir.display());
    let mut ret = OsString::new();
    while dirc.is_some() {
        if !ret.is_empty() {
            ret.push("/");
        }
        ret.push("..");
        dirc = diri.next();
    }
    while let Some(r) = relc {
        if !ret.is_empty() {
            ret.push("/");
        }
        ret.push(r);
        relc = reli.next();
    }
    if ret.is_empty() {
        ret.push(".");
    }
    ret
}

fn configure(build_dir: &Path, conf_line: &OsStr) {
    let mut conf = Command::new("sh");
    conf.current_dir(&build_dir).arg("-c").arg(conf_line);
    execute(conf);
}

fn make_and_check(build_dir: &Path, jobs: &OsStr, check: bool) {
    let mut make = Command::new("make");
    make.current_dir(build_dir).arg("-j").arg(jobs);
    execute(make);
    if check {
        let mut make_check = Command::new("make");
        make_check
            .current_dir(build_dir)
            .arg("-j")
            .arg(jobs)
            .arg("check");
        execute(make_check);
    }
}

fn copy_file(src: &Path, dst: &Path) {
    fs::copy(&src, &dst).unwrap_or_else(|_| {
                                            panic!("Unable to copy {} -> {}",
                                                   src.display(),
                                                   dst.display());
                                        });
}

fn symlink(dir: &Path, link: &OsStr, name: Option<&OsStr>) {
    let mut c = Command::new("ln");
    c.current_dir(dir).arg("-s").arg(link);
    if let Some(name) = name {
        c.arg(name);
    }
    execute(c);
}

fn execute(mut command: Command) {
    println!("$ {:?}", command);
    let status =
        command
            .status()
            .unwrap_or_else(|_| panic!("Unable to execute: {:?}", command));
    if !status.success() {
        if let Some(code) = status.code() {
            panic!("Program failed with code {}: {:?}", code, command);
        } else {
            panic!("Program failed: {:?}", command);
        }
    }
}

fn open(name: &Path) -> BufReader<File> {
    let file =
        File::open(name)
            .unwrap_or_else(|_| panic!("Cannot open file: {}", name.display()));
    BufReader::new(file)
}

fn create(name: &Path) -> BufWriter<File> {
    let file =
        File::create(name).unwrap_or_else(|_| {
                                              panic!("Cannot create file: {}",
                                                     name.display())
                                          });
    BufWriter::new(file)
}

fn read_line(reader: &mut BufReader<File>,
             buf: &mut String,
             name: &Path)
             -> usize {
    reader
        .read_line(buf)
        .unwrap_or_else(|_| panic!("Cannot read from: {}", name.display()))
}

fn write(writer: &mut BufWriter<File>, buf: &str, name: &Path) {
    writer
        .write(buf.as_bytes())
        .unwrap_or_else(|_| panic!("Cannot write to: {}", name.display()));
}

fn flush(writer: &mut BufWriter<File>, name: &Path) {
    writer
        .flush()
        .unwrap_or_else(|_| panic!("Cannot write to: {}", name.display()));
}
