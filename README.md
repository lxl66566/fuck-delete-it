# Fuck, delete it!

English | [简体中文](./docs/README.zh-CN.md)

Kill all processes occupying a file/folder, and enforce deletion. [Inspired by](https://t.me/withabsolutex/1537)

Have you ever encountered a situation where you can't delete a file/folder because it's being used by another program? This can be annoying on Windows: you may have to search for handles in the _Resource Monitor_, find the process, and then kill it. This program simplifies that.

## Installation

There are several different methods to install this program, and you can choose any of them.

- Download the file from [Releases](https://github.com/lxl66566/fuck-delete-it/releases) and unzip it, then place it in `C:\Windows\System32` or any directory that exists in your `Path`.
- Use [bpm](https://github.com/lxl66566/bpm):
  ```sh
  bpm i fuck-delete-it -b fdi -q
  ```
- Use [scoop](https://scoop.sh/):
  ```sh
  scoop install https://raw.githubusercontent.com/lxl66566/fuck-delete-it/main/fuck-delete-it.json
  ```

<!-- - Use cargo:
  ```sh
  cargo install fuck-delete-it
  ``` -->

## Usage

Command line:

```sh
fdi <PATH> [-y]
```

You can also directly drag and drop the file/folder into the command line window, and the path will be automatically filled in.

For more information, please check `fdi -h`.
