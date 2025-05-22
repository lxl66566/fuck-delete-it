# Fuck, delete it!

[English](../README.md) | 简体中文

杀死所有占用文件/文件夹的进程，强制执行删除。[灵感来源](https://t.me/withabsolutex/1537)

你是否遇到这样的情况：无法删除文件/文件夹，因为其他程序正在使用它？在 Windows 上这很烦人：需要去 _资源监视器_ 里搜索句柄，找到进程，再结束进程。此程序简化了这一步骤。

## 安装

有几种不同方法可以安装此程序，你可以选择其中任意一种。

- 在 [Releases](https://github.com/lxl66566/fuck-delete-it/releases) 中下载文件并解压，放入 `C:\Windows\System32` 或 `Path` 环境变量中的任何目录下。
- 使用 [bpm](https://github.com/lxl66566/bpm)：
  ```sh
  bpm i fuck-delete-it -q
  ```
- 使用 [cargo-binstall](https://github.com/cargo-bins/cargo-binstall)：
  ```sh
  cargo binstall fuck-delete-it
  ```

## 使用

安装后，直接运行该程序，其会将 `FUCK, DELETE IT!` 条目加入到文件/文件夹的右键菜单中。只需要右键单击文件/文件夹，点击该菜单项即可执行强制删除。

也可以使用命令行进行删除：

```sh
fdi [PATH] [-y] [-u]
```

可以直接将文件/文件夹拖入命令行窗口，路径会被自动填入。

更多信息请查看 `fdi -h`。
