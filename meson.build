project('thane', 'rust',
  version: '0.1.0',
  license: 'AGPL-3.0-or-later',
)

datadir = get_option('datadir')
prefix = get_option('prefix')

# Install desktop file
install_data(
  'data/com.thane.app.desktop',
  install_dir: datadir / 'applications',
)

# Install AppStream metadata
install_data(
  'data/com.thane.app.metainfo.xml',
  install_dir: datadir / 'metainfo',
)

# Install GSettings schema
install_data(
  'data/com.thane.app.gschema.xml',
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

# Install scalable icon
install_data(
  'data/icons/hicolor/scalable/apps/com.thane.app.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
)

# Install man pages
install_data(
  'doc/thane.1',
  install_dir: get_option('mandir') / 'man1',
)
install_data(
  'doc/thane-cli.1',
  install_dir: get_option('mandir') / 'man1',
)

# Install systemd user service
install_data(
  'data/thane.service',
  install_dir: prefix / 'lib' / 'systemd' / 'user',
)

# Compile GSettings schemas
gnome = import('gnome')
gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
