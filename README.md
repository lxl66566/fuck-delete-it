# Fuck, delete it!

English | [简体中文](./docs/README.zh-CN.md)

Kill all processes occupying a file/folder and force deletion. [Inspiration](https://t.me/withabsolutex/1537)

Have you ever encountered a situation where you can't delete a file/folder because it's being used by another program? On Windows, this is annoying: you need to go to _Resource Monitor_, search for handles, find the process, and then terminate it. This program simplifies the process.

## Installation

There are several ways to install this program, and you can choose any of them.

- Download the file from [Releases](https://github.com/lxl66566/fuck-delete-it/releases), unzip it, and place it in `C:\Windows\System32` or any directory in the `Path` environment variable.
- Use [bpm](https://github.com/lxl66566/bpm):
  ```sh
  bpm i fuck-delete-it -q
  ```
- Use [cargo-binstall](https://github.com/cargo-bins/cargo-binstall):
  ```sh
  cargo binstall fuck-delete-it
  ```

## Usage

After installation, simply run the program, and it will add the `FUCK, DELETE IT!` option to the right-click context menu for files/folders. Just right-click the file/folder and select this menu item to force deletion.

You can also use the command line to delete:

```sh
fdi [PATH] [-y] [-u]
```

Alternatively, you can directly drag the file/folder into the command line window, and the path will be automatically filled in.

For more information, check `fdi -h`.
