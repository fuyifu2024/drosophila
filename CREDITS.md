# 数据来源与致谢

本项目沿用原本地项目所记录的 FlyWire / FAFB v783 数据来源。
连接组、神经元坐标、细胞标注及可选脑解剖网格来自下列研究工作；
轻量版仅默认展示神经元点云，不加载脑壳与神经毡表面。

## 连接组与坐标:FlyWire / FAFB v783

- **FlyWire Consortium** — 全脑连接组的重建与校对,由普林斯顿、剑桥 MRC LMB、
  哈佛等机构的数百人合作完成。
- **FAFB 图像数据**:Zheng et al., *A Complete Electron Microscopy Volume of the Brain
  of Adult Female Drosophila melanogaster*, **Cell** 174, 730–743 (2018).
  doi:10.1016/j.cell.2018.06.019

引用这份连接组时应当引用:

- Dorkenwald, S. et al. *Neuronal wiring diagram of an adult brain.*
  **Nature** 634, 124–138 (2024). doi:10.1038/s41586-024-07558-y
- Schlegel, P. et al. *Whole-brain annotation and multi-connectome cell typing of
  Drosophila.* **Nature** (2024). doi:10.1038/s41586-024-07686-5
- Matsliah, A. et al. *Neuronal parts list and wiring diagram for a visual system.*
  **Nature** (2024). doi:10.1038/s41586-024-07981-1
- Lin, A. et al. *Network statistics of the whole-brain connectome of Drosophila.*
  **Nature** (2024). doi:10.1038/s41586-024-07968-y

数据经 **FlyWire Codex** 分发:<https://codex.flywire.ai>

## 三维网格:脑壳与神经毡

`data/brain_shell.obj` 和 `data/neuropils/*.obj` 来自
**Lee Lab / Brain and Nerve Cord Fly Connectome** 公开数据桶:

<https://storage.googleapis.com/lee-lab_brain-and-nerve-cord-fly-connectome/compiled_data/fafb_783/obj/>

## 许可:**CC BY-NC 4.0(非商业)**

FlyWire 的数据以 **Creative Commons Attribution-NonCommercial 4.0** 发布。
<https://creativecommons.org/licenses/by-nc/4.0/>

数据的使用和再分发须保留署名，并遵守原始 CC BY-NC 4.0 许可。

本项目自身的代码由作者以 MIT 许可发布;数据部分不适用,仍受 CC BY-NC 4.0 约束。

## 思路借鉴

原项目记录：坐标系与 LIF 内核的写法参考了
[gnat](https://github.com/lubabs770/gnat)。本项目是独立的 Rust 仿真与浏览器 3D 查看器。
