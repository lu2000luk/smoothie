from pathlib import Path
import shutil


CRUN_VERSION = "1.19.1"
PREFIX = Path("/usr/local")
PKG_DIR = PREFIX / "lib" / "pkgconfig"
INCLUDE_DIR = PREFIX / "include"


def write_pkgconfig():
    PKG_DIR.mkdir(parents=True, exist_ok=True)
    pc = PKG_DIR / "libcrun.pc"
    pc.write_text(
        f"""prefix={PREFIX}
exec_prefix=${{prefix}}
libdir=${{exec_prefix}}/lib
includedir=${{prefix}}/include

Name: libcrun
Description: OCI runtime library
Version: {CRUN_VERSION}
Libs: -L${{libdir}} -lcrun -lyajl -lseccomp -lcap
Cflags: -I${{includedir}}
""",
        encoding="utf-8",
    )
    print(f"Wrote {pc}")


def install_headers(source: Path):
    (INCLUDE_DIR / "libcrun").mkdir(parents=True, exist_ok=True)
    (INCLUDE_DIR / "ocispec").mkdir(parents=True, exist_ok=True)

    for hdr in (source / "src" / "libcrun").glob("*.h"):
        shutil.copy2(hdr, INCLUDE_DIR / "libcrun" / hdr.name)

    if (source / "config.h").exists():
        shutil.copy2(source / "config.h", INCLUDE_DIR / "config.h")

    for hdr in (source / "libocispec").rglob("*.h"):
        shutil.copy2(hdr, INCLUDE_DIR / "ocispec" / hdr.name)

    print("Installed headers to /usr/local/include")


def main():
    build = Path("/tmp/crun")
    install_headers(build)
    write_pkgconfig()


if __name__ == "__main__":
    main()
