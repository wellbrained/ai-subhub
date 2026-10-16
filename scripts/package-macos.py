"""Create an unsigned test .app ZIP; standard-library-only, usable on Windows."""
import argparse
import pathlib
import plistlib
import shutil
import stat
import zipfile


def package(published, output, version, icon):
    app = output / "AI SubHub.app"
    if app.exists():
        raise SystemExit(f"Bundle already exists: {app}. Choose a fresh output directory.")
    macos = app / "Contents" / "MacOS"
    shutil.copytree(published, macos)
    resources = app / "Contents" / "Resources"
    resources.mkdir()
    shutil.copy2(icon, resources / "hub.icns")
    info = dict(CFBundleName="AI SubHub", CFBundleDisplayName="AI SubHub",
                CFBundleIdentifier="org.aisubhub.app", CFBundleVersion=version,
                CFBundleShortVersionString=version, CFBundleExecutable="AI.SubHub",
                CFBundlePackageType="APPL", CFBundleIconFile="hub.icns",
                LSMinimumSystemVersion="15.0", LSUIElement=True,
                NSHighResolutionCapable=True,
                NSAppleEventsUsageDescription="AI SubHub opens Terminal to run the CLI profile and project you select.")
    with (app / "Contents" / "Info.plist").open("wb") as stream:
        plistlib.dump(info, stream)
    executable = macos / "AI.SubHub"
    if not executable.is_file():
        raise SystemExit("Published Mac apphost is missing.")
    archive = pathlib.Path(str(output) + ".zip")
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zipped:
        for item in sorted(app.rglob("*")):
            relative = item.relative_to(output).as_posix() + ("/" if item.is_dir() else "")
            entry = zipfile.ZipInfo(relative)
            entry.create_system = 3  # macOS must see Unix executable permissions.
            mode = 0o755 if item.is_dir() or item == executable or item.suffix == ".dylib" else 0o644
            entry.external_attr = ((stat.S_IFDIR if item.is_dir() else stat.S_IFREG) | mode) << 16
            if item.is_dir():
                entry.external_attr |= 0x10
            entry.compress_type = zipfile.ZIP_DEFLATED
            zipped.writestr(entry, b"" if item.is_dir() else item.read_bytes())
    print(archive)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("published", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("version")
    parser.add_argument("icon", type=pathlib.Path)
    args = parser.parse_args()
    package(args.published, args.output, args.version, args.icon)
