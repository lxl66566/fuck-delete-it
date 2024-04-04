# Fuck, delete it!

[English](../README.md) | 简体中文

杀死所有占用文件/文件夹的进程，强制执行删除。[灵感来源](https://t.me/withabsolutex/1537)

您是否遇到这样的情况：无法删除文件/文件夹，因为其他程序正在使用它？在 Windows 上这很烦人：您可能要去 _资源监视器_ 里搜索句柄，找到进程，再结束进程。此程序简化了这一步骤。

## 安装

有几种不同方法可以安装此程序，您可以选择其中任意一种。

- 在 [Releases](https://github.com/lxl66566/fuck-delete-it/releases) 中下载文件并解压，放入 `C:\Windows\System32` 或任意存在于 `Path` 的目录下。
- 使用 [bpm](https://github.com/lxl66566/bpm)：
  ```sh
  bpm i fuck-delete-it -b fdi -q
  ```
- 使用 [scoop](https://scoop.sh/)：
  ```sh
  scoop install https://raw.githubusercontent.com/lxl66566/fuck-delete-it/main/fuck-delete-it.json
  ```

<!-- - 使用 cargo：
  ```sh
  cargo install fuck-delete-it
  ``` -->

## 使用

命令行：

```sh
fdi <PATH> [-y]
```

您也可以直接将文件/文件夹拖入命令行窗口，路径会被自动填入。

更多信息请查看 `fdi -h`。
