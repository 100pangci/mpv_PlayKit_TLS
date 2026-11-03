# Shader 许可证审计

本文件是 `manifests/shader-audit.json` 的人工可读版本，记录基线
`hooke007/mpv_PlayKit` tag `20260510`
(commit `1ad89a52e8dab222b5628ab835bb0803995ead3a`) 中的每个 GLSL 文件、随后补入的
上游文件，以及它们在本发行版中的处置。机器可读 JSON 才是权威记录；
两者由 `python -m unittest discover -s tests -v` 和 `build.verify` 交叉校验。

## 结论

- 纳入 **13 个组件、12 个组件级 family、191 个 GLSL target**；审计按功能细分后
  为 14 个 family（`deband`、`edi`、`etc`、`guided`、`raisr` 等与组件级分组并列）。
- 纳入的代码许可证全部为 MIT、BSD-2-Clause、LGPL-2.1-or-later、
  LGPL-3.0-or-later 或 GPL-3.0-only；每个文件固定到完整 commit 和 SHA-256。
- AI shader 的模型权重单独记录：来源仓库、固定 commit、权重路径、模型许可证
  和随包许可证文本。代码许可证不被假定覆盖权重。
- 不满足“来源、代码许可证、模型/数据许可证三者均可证明”的条目全部排除，
  包括 NonCommercial 模型、无授权转换、Shadertoy/Pastebin/Gist 片段和
  仅存在于 hooke007 派生 port 中的文件。

状态统计（共 442 条审计记录）：

| 状态 | 全部 | 纳入 | 未纳入 |
| --- | ---: | ---: | ---: |
| `SAFE` | 93 | 93 | 0 |
| `SAFE_WITH_NOTICE` | 156 | 103 | 53 |
| `NONCOMMERCIAL` | 9 | 0 | 9 |
| `CLEAN_ROOM_REIMPLEMENT_CANDIDATE` | 62 | 0 | 62 |
| `UNKNOWN` | 122 | 0 | 122 |

## 纳入组件

| id | family | 真正上游 | 固定 commit | 代码许可证 | 模型许可证 | 文件数 | 状态 |
| --- | --- | --- | --- | --- | --- | ---: | --- |
| `acnet` | `acnet` | `TianZerL/ACNetGLSL` | `c7d2d8dbb5364c550e5bfb738860fc9bcc0ea424` | MIT | MIT | 33 | SAFE |
| `adaptive-sharpen-lite` | `sharpen` | `hooke007/mpv_PlayKit` | `1ad89a52e8dab222b5628ab835bb0803995ead3a` | BSD-2-Clause | — | 3 | SAFE_WITH_NOTICE |
| `aiupscale` | `aiupscale` | `Alexkral/AviSynthAiUpscale` | `d04cf8154e4ba9914f3ead0dec9f7a4a7df7369f` | GPL-3.0-only | GPL-3.0-only | 36 | SAFE_WITH_NOTICE |
| `an3223-denoise` | `nlmeans` | `AN3223/dotfiles` | `4413db6c1b47aa70ffce40c5a91158e518881718` | LGPL-2.1-or-later | — | 14 | SAFE_WITH_NOTICE |
| `anime4k-gan` | `anime4k` | `bloc97/Anime4K` | `8e39551ce96ed172605c89b7dd8be855b5502cc9` | MIT | MIT | 10 | SAFE |
| `anime4k-v4` | `anime4k` | `bloc97/Anime4K` | `4029bf701ecaa15f163cdc49cffe5501c1acf410` | MIT | MIT | 39 | SAFE |
| `artcnn` | `artcnn` | `Artoriuz/ArtCNN` | `0837e5326e710f49e2ab0a8d995da0708047442e` | MIT | MIT | 6 | SAFE |
| `chroma_from_luma_prediction` | `cfl` | `Artoriuz/glsl-chroma-from-luma-prediction` | `066dff964b83aa34e893de25d590874bf8c8e94f` | MIT | — | 2 | SAFE |
| `cunny` | `cunny` | `funnyplanter/CuNNy` | `906031bb00c15dd6a6bbbaa21c0eb0b724ca8437` | LGPL-3.0-or-later | LGPL-3.0-or-later | 18 | SAFE_WITH_NOTICE |
| `fsrcnnx` | `fsrcnnx` | `igv/FSRCNN-TensorFlow` | `1.1` | LGPL-3.0-or-later | LGPL-3.0-or-later | 1 | SAFE_WITH_NOTICE |
| `joint_bilateral` | `joint_bilateral` | `Artoriuz/glsl-joint-bilateral` | `0b4b4a65f41d57ecee60441f632ab33ad23c6188` | MIT | — | 2 | SAFE |
| `mpv-prescalers` | `prescalers` | `bjin/mpv-prescalers` | `b3f0a59d68f33b7162051ea5970a5169558f0ea2` | LGPL-3.0-or-later | LGPL-3.0-or-later | 26 | SAFE_WITH_NOTICE |
| `pixel_clipper` | `pixel_clipper` | `Artoriuz/glsl-pixel-clipper` | `92aa62129d0fd2ce482de52b7aa0fe91e99af1b5` | MIT | — | 1 | SAFE |

### 模型/数据权重

| 组件 | 模型 | 作者 | 权重来源 | 权重路径 | 模型许可证 | 随包文本 |
| --- | --- | --- | --- | --- | --- | --- |
| `acnet` | ACNet / ARNet trained weights (embedded in GLSL) | TianZer | `TianZerL/ACNetGLSL@c7d2d8dbb5364c550e5bfb738860fc9bcc0ea424` | `glsl` | MIT | `LICENSE` |
| `aiupscale` | AiUpscale Fast/Medium/HQ models | Alexkral | `Alexkral/AviSynthAiUpscale@d04cf8154e4ba9914f3ead0dec9f7a4a7df7369f` | `Shaders` | GPL-3.0-only | `LICENSE` |
| `anime4k-gan` | Anime4K GAN models (weights embedded in GLSL) | bloc97 | `bloc97/Anime4K@8e39551ce96ed172605c89b7dd8be855b5502cc9` | `glsl` | MIT | `LICENSE` |
| `anime4k-v4` | Anime4K v4 CNN models (weights embedded in GLSL) | bloc97 | `bloc97/Anime4K@4029bf701ecaa15f163cdc49cffe5501c1acf410` | `glsl` | MIT | `LICENSE` |
| `artcnn` | ArtCNN C4F16 / C4F32 ONNX weights | João Chrisóstomo | `Artoriuz/ArtCNN@0837e5326e710f49e2ab0a8d995da0708047442e` | `ONNX` | MIT | `LICENSE` |
| `cunny` | CuNNy pretrained pickle weights | funnyplanter | `funnyplanter/CuNNy@906031bb00c15dd6a6bbbaa21c0eb0b724ca8437` | `pretrained` | LGPL-3.0-or-later | `LICENSE` |
| `fsrcnnx` | FSRCNNX x2 16-0-4-1 weights (embedded in release shader) | igv | `igv/FSRCNN-TensorFlow@1aa11ab0e1fc12741fdb84cef31da5619a478670` | `FSRCNNX_x2_16-0-4-1.glsl` | LGPL-3.0-or-later | `LGPL-3.0-or-later.txt` |
| `mpv-prescalers` | nnedi3 neural weights and RAVU LUT data (embedded in .hook) | bjin and upstream authors | `bjin/mpv-prescalers@b3f0a59d68f33b7162051ea5970a5169558f0ea2` | `.` | LGPL-3.0-or-later | `LICENSE` |

无需单独模型权重的非 AI shader（CfL、JointBilateral、PixelClipper、
adaptive sharpen、AN3223 denoise/guided）只有代码许可证；其上游数据或
推导关系仍记录在 `derived_from` 或组件 notes 中。

## 未纳入条目

以下条目保留在审计里，但 `included_in_tls` 为 false。分组按状态；
`reason` 与 `replacement_source` 的完整值见 JSON。`SAFE_WITH_NOTICE` 中未纳入
的条目（旧 ArtCNN 代、FSRCNNX 变体、RAVU RGB/YUV 变体、GEGL unsharp 等）
并不是许可证有问题，而是为避免重复目标和包体过大而刻意只选一部分；它们的
许可证链仍记录在审计中。

### NONCOMMERCIAL（9 条）

| 路径 | family | 代码许可证 | 模型许可证 | 处置/原因 |
| --- | --- | --- | --- | --- |
| `Ani/Ani4Kv2_ArtCNN_C4F32_i2.glsl` | `ani4k` | MIT | CC-BY-NC-4.0 | The code is MIT, but the Ani4Kv2/AniSD model weights are CC BY-NC 4.0. NonCommercial terms are incompatible with this freely redistributable package. |
| `Ani/Ani4Kv2_ArtCNN_C4F32_i2_CMP.glsl` | `ani4k` | MIT | CC-BY-NC-4.0 | The code is MIT, but the Ani4Kv2/AniSD model weights are CC BY-NC 4.0. NonCommercial terms are incompatible with this freely redistributable package. |
| `Ani/Ani4Kv2_ArtCNN_C4F32_i2_dx.glsl` | `ani4k` | MIT | CC-BY-NC-4.0 | The code is MIT, but the Ani4Kv2/AniSD model weights are CC BY-NC 4.0. NonCommercial terms are incompatible with this freely redistributable package. |
| `Ani/AniSD_ArtCNN_C4F32_i4.glsl` | `ani4k` | MIT | CC-BY-NC-4.0 | The code is MIT, but the Ani4Kv2/AniSD model weights are CC BY-NC 4.0. NonCommercial terms are incompatible with this freely redistributable package. |
| `Ani/AniSD_ArtCNN_C4F32_i4_CMP.glsl` | `ani4k` | MIT | CC-BY-NC-4.0 | The code is MIT, but the Ani4Kv2/AniSD model weights are CC BY-NC 4.0. NonCommercial terms are incompatible with this freely redistributable package. |
| `Ani/AniSD_ArtCNN_C4F32_i4_dx.glsl` | `ani4k` | MIT | CC-BY-NC-4.0 | The code is MIT, but the Ani4Kv2/AniSD model weights are CC BY-NC 4.0. NonCommercial terms are incompatible with this freely redistributable package. |
| `ESRGAN/the_database_AnimeJaNaiV2L1_x2.glsl` | `esrgan` | — | CC-BY-NC-SA-4.0 | the_database AnimeJaNai models are CC BY-NC-SA 4.0; NonCommercial. |
| `ESRGAN/the_database_AnimeJaNaiV3L1_HD_x2.glsl` | `esrgan` | — | CC-BY-NC-SA-4.0 | the_database AnimeJaNai models are CC BY-NC-SA 4.0; NonCommercial. |
| `ESRGAN/the_database_AnimeJaNaiV3L1_sharp_HD_x2.glsl` | `esrgan` | — | CC-BY-NC-SA-4.0 | the_database AnimeJaNai models are CC BY-NC-SA 4.0; NonCommercial. |

### CLEAN_ROOM_REIMPLEMENT_CANDIDATE（62 条）

| 路径 | family | 代码许可证 | 模型许可证 | 处置/原因 |
| --- | --- | --- | --- | --- |
| `AA/Intel_CMAA2_lite.glsl` | `aa` | Apache-2.0 | — | CMAA2 is Apache-2.0, but only HLSL is published upstream and this GLSL adaptation has no independent provenance. Clean-room candidate.；替代来源：clean-room reimplementation |
| `AA/Intel_CMAA2_lite_md.glsl` | `aa` | Apache-2.0 | — | CMAA2 is Apache-2.0, but only HLSL is published upstream and this GLSL adaptation has no independent provenance. Clean-room candidate.；替代来源：clean-room reimplementation |
| `AA/SMAA_lite_RT.glsl` | `aa` | MIT | — | SMAA itself is MIT, but the reference file is an unlicensed GLSL port. The algorithm can be reimplemented cleanly; the hooke007 file is not copied.；替代来源：clean-room reimplementation |
| `AA/SMAA_lite_md_RT.glsl` | `aa` | MIT | — | SMAA itself is MIT, but the reference file is an unlicensed GLSL port. The algorithm can be reimplemented cleanly; the hooke007 file is not copied.；替代来源：clean-room reimplementation |
| `AMD/AMD_BCAS_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_CAS_AIO_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_CAS_luma_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_CAS_scl_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_EASU.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_EASU_luma_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_RCAS_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_RCAS_luma_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_RCAS_scl_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_lite_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `AMD/AMD_FSR1_luma_RT.glsl` | `amd` | MIT | — | Official CAS/FSR1 sources are MIT, but these are unverified GLSL ports. Clean-room reimplementation is the safe route.；替代来源：clean-room reimplementation |
| `Canvas/AMD_FidelityFX_Lens_RT.glsl` | `canvas` | MIT | — | AMD lens distortion algorithm is MIT upstream, but this GLSL port is unverified.；替代来源：clean-room reimplementation |
| `Canvas/box_crop_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/box_pad_ada.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/flip_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/keystone_adjust_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/pan_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/rotateR_FX_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/rotate_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `Canvas/zoom_RT.glsl` | `canvas` | — | — | Simple geometric transform; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `color/color_alt_luma.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/color_alt_rgb.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/color_hold_RT.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/color_temperature_RT.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorlevel_expand.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorlevel_expand_chroma.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorlevel_expand_luma.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorlevel_shrink.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorlevel_shrink_chroma.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorlevel_shrink_luma.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `color/colorprim_601_to_709.glsl` | `color` | — | — | Simple color matrix / level transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `DPID/DPID_RT.glsl` | `dpid` | BSD-3-Clause | — | DPID is BSD-3-Clause in CUDA/MATLAB, but this GLSL port has no verified grant.；替代来源：clean-room reimplementation |
| `DPID/DPID_lite_RT.glsl` | `dpid` | BSD-3-Clause | — | DPID is BSD-3-Clause in CUDA/MATLAB, but this GLSL port has no verified grant.；替代来源：clean-room reimplementation |
| `EDI/DCCI_RT.glsl` | `edi` | — | — | DCCI is a published algorithm; no verified shader code license.；替代来源：clean-room reimplementation |
| `EDI/DCCI_luma_RT.glsl` | `edi` | — | — | DCCI is a published algorithm; no verified shader code license.；替代来源：clean-room reimplementation |
| `EDI/NEDI_Fast_luma_RT.glsl` | `edi` | LGPL-3.0-or-later | — | NEDI MPDN implementation is LGPL, but this GLSL adaptation is unverified.；替代来源：clean-room reimplementation |
| `EDI/NEDI_MPDN_luma.glsl` | `edi` | LGPL-3.0-or-later | — | NEDI MPDN implementation is LGPL, but this GLSL adaptation is unverified.；替代来源：clean-room reimplementation |
| `mirror/mirror_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `mirror/mirror_fill_sp_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `mirror/mirror_sp_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `plane/plane_block_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `plane/plane_shift_rgb_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `plane/plane_shift_yuv_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `plane/plane_splitV2_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `plane/plane_split_RT.glsl` | `geometry` | — | — | Simple geometric operation; can be reimplemented from scratch.；替代来源：clean-room reimplementation |
| `MagicKernel/Magic_Kernel_Classic.glsl` | `magic_kernel` | — | — | Magic Kernel is described by a published paper; no verified shader code license.；替代来源：clean-room reimplementation |
| `MagicKernel/Magic_Kernel_Sharp2013.glsl` | `magic_kernel` | — | — | Magic Kernel is described by a published paper; no verified shader code license.；替代来源：clean-room reimplementation |
| `MagicKernel/Magic_Kernel_Sharp2021.glsl` | `magic_kernel` | — | — | Magic Kernel is described by a published paper; no verified shader code license.；替代来源：clean-room reimplementation |
| `MagicKernel/Magic_Kernel_Sharp2021_lut.glsl` | `magic_kernel` | — | — | Magic Kernel is described by a published paper; no verified shader code license.；替代来源：clean-room reimplementation |
| `NV/NVScaler_RT.glsl` | `nvidia` | MIT | — | NIS is MIT, but only Vulkan/HLSL sources exist; this GLSL port is unverified.；替代来源：clean-room reimplementation |
| `NV/NVScaler_luma_RT.glsl` | `nvidia` | MIT | — | NIS is MIT, but only Vulkan/HLSL sources exist; this GLSL port is unverified.；替代来源：clean-room reimplementation |
| `NV/NVSharpen_RT.glsl` | `nvidia` | MIT | — | NIS is MIT, but only Vulkan/HLSL sources exist; this GLSL port is unverified.；替代来源：clean-room reimplementation |
| `NV/NVSharpen_luma_RT.glsl` | `nvidia` | MIT | — | NIS is MIT, but only Vulkan/HLSL sources exist; this GLSL port is unverified.；替代来源：clean-room reimplementation |
| `NV/NVSharpen_scl_RT.glsl` | `nvidia` | MIT | — | NIS is MIT, but only Vulkan/HLSL sources exist; this GLSL port is unverified.；替代来源：clean-room reimplementation |
| `Pixel-art/pixellate_RT.glsl` | `pixelart` | — | — | Trivial pixelation transform; can be reimplemented cleanly.；替代来源：clean-room reimplementation |
| `SSim/SSimDownscaler_RT.glsl` | `ssim` | LGPL-3.0-or-later | — | SSim MPDN code is LGPL, but this GLSL port lacks verified provenance.；替代来源：clean-room reimplementation |
| `SSim/SSimSuperRes_RT.glsl` | `ssim` | LGPL-3.0-or-later | — | SSim MPDN code is LGPL, but this GLSL port lacks verified provenance.；替代来源：clean-room reimplementation |

### UNKNOWN（122 条）

| 路径 | family | 代码许可证 | 模型许可证 | 处置/原因 |
| --- | --- | --- | --- | --- |
| `AA/AXAA.glsl` | `aa` | — | — | AXAA port has no explicit license or verified original upstream. |
| `AA/AXAA_md.glsl` | `aa` | — | — | AXAA port has no explicit license or verified original upstream. |
| `AA/DLAA_RT.glsl` | `aa` | — | — | NVIDIA DLAA/FAAA shader conversion has no explicit redistribution grant. |
| `AA/DLAA_md_RT.glsl` | `aa` | — | — | NVIDIA DLAA/FAAA shader conversion has no explicit redistribution grant. |
| `AA/FAAA_RT.glsl` | `aa` | — | — | NVIDIA DLAA/FAAA shader conversion has no explicit redistribution grant. |
| `AA/FAAA_md_RT.glsl` | `aa` | — | — | NVIDIA DLAA/FAAA shader conversion has no explicit redistribution grant. |
| `ACNet/arnet_b24_hdn.glsl` | `acnet` | — | — | License or weight provenance could not be proven from the original upstream. |
| `ACNet/arnet_b24_le.glsl` | `acnet` | — | — | License or weight provenance could not be proven from the original upstream. |
| `ACNet/arnet_b48_hdn.glsl` | `acnet` | — | — | License or weight provenance could not be proven from the original upstream. |
| `ACNet/arnet_b48_le.glsl` | `acnet` | — | — | License or weight provenance could not be proven from the original upstream. |
| `ACNet/arnet_b4_hdn.glsl` | `acnet` | — | — | License or weight provenance could not be proven from the original upstream. |
| `ACNet/arnet_b4_le.glsl` | `acnet` | — | — | License or weight provenance could not be proven from the original upstream. |
| `Anime4K/Anime4K_AIO_optQ.glsl` | `anime4k` | MIT | — | hooke007 composite profile, not an upstream Anime4K file; no independent grant. |
| `Anime4K/Anime4K_Restore_GAN_UL_CC.glsl` | `anime4k` | MIT | — | hooke007 CC variant is not present in the pinned upstream GAN commit. |
| `Anime4K/Anime4K_Upscale_GAN_x2_UL.glsl` | `anime4k` | MIT | — | GAN x2 UL variant is not present in the pinned upstream GAN commit. |
| `Canvas/ASCII_art.glsl` | `canvas` | — | — | Shadertoy-derived shader without an explicit license or author grant. |
| `Canvas/MandelbrotSet_lite_FX_RT.glsl` | `canvas` | — | — | Shadertoy-derived shader without an explicit license or author grant. |
| `Canvas/MandelbrotSet_p3d_FX_RT.glsl` | `canvas` | — | — | Shadertoy-derived shader without an explicit license or author grant. |
| `Canvas/NLS_RT.glsl` | `canvas` | — | — | NLS / kNLS shader has no verified original license. |
| `Canvas/Seascape_FX_RT.glsl` | `canvas` | — | — | Shadertoy-derived shader without an explicit license or author grant. |
| `Canvas/kNLS_RT.glsl` | `canvas` | — | — | NLS / kNLS shader has no verified original license. |
| `CfL/KrigBilateral.glsl` | `cfl` | — | — | KrigBilateral provenance and license could not be proven; not copied. |
| `CfL/kACfL_RT.glsl` | `cfl` | — | — | hooke007 kACfL variant has no explicit independent license. |
| `color/color_BlindAid_RT.glsl` | `color` | — | — | Color-blind aid shader has no explicit license. |
| `CuNNy/CuNNy_2x12_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_2x12_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_3x12_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_3x12_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x12_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x12_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x16_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x16_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x24_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x24_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x32_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_4x32_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_8x32_DS_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_fast_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_faster_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `CuNNy/CuNNy_veryfast_SOFT_vk.glsl` | `cunny` | LGPL-3.0-or-later | LGPL-3.0-or-later | hooke007 Vulkan-specific CuNNy variant; not present in the upstream mpv shader set. |
| `Deband/kgradfun_RT.glsl` | `deband` | — | — | Deband port's upstream license chain is not established. |
| `Deband/neo_f3kdb_luma_RT.glsl` | `deband` | — | — | Deband port's upstream license chain is not established. |
| `Deband/neo_f3kdb_rgb_RT.glsl` | `deband` | — | — | Deband port's upstream license chain is not established. |
| `dehaasn/dehaasn.glsl` | `deband` | — | — | Shader adaptation has no explicit license or verified original repository. |
| `dehaasn/dehaasn_ads_luma_RT.glsl` | `deband` | — | — | Shader adaptation has no explicit license or verified original repository. |
| `dehaasn/dehaasn_ads_rgb_RT.glsl` | `deband` | — | — | Shader adaptation has no explicit license or verified original repository. |
| `deint/deint_swa.glsl` | `deint` | — | — | Shader adaptation has no explicit license or verified original repository. |
| `deint/deint_swa_RT.glsl` | `deint` | — | — | Shader adaptation has no explicit license or verified original repository. |
| `deint/deint_swa_luma.glsl` | `deint` | — | — | Shader adaptation has no explicit license or verified original repository. |
| `eq/eq_ColorBlindSim_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_HDRR_faux_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_HDRR_kfaux_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_clarity_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_classic_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_ntsc_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_rgb_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_temperature_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/eq_yiq_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `eq/kVibrance_RT.glsl` | `eq` | — | — | Equalizer shader has no explicit license or verified original source. |
| `ESPCN/Lornatang_espcn_T91_x2.glsl` | `espcn` | Apache-2.0 | — | ESPCN code is Apache-2.0, but pretrained weights either lack a separate weight license or come from an unlicensed dataset. Weights are not redistributable on this evidence. |
| `ESPCN/Lornatang_espcn_T91_x3.glsl` | `espcn` | Apache-2.0 | — | ESPCN code is Apache-2.0, but pretrained weights either lack a separate weight license or come from an unlicensed dataset. Weights are not redistributable on this evidence. |
| `ESPCN/Lornatang_espcn_T91_x4.glsl` | `espcn` | Apache-2.0 | — | ESPCN code is Apache-2.0, but pretrained weights either lack a separate weight license or come from an unlicensed dataset. Weights are not redistributable on this evidence. |
| `ESPCN/aoxo_superres_best_x2_CC.glsl` | `espcn` | Apache-2.0 | — | ESPCN code is Apache-2.0, but pretrained weights either lack a separate weight license or come from an unlicensed dataset. Weights are not redistributable on this evidence. |
| `ESPCN/aoxo_superres_best_x3_CC.glsl` | `espcn` | Apache-2.0 | — | ESPCN code is Apache-2.0, but pretrained weights either lack a separate weight license or come from an unlicensed dataset. Weights are not redistributable on this evidence. |
| `ESRGAN/W2xEX_AnimeVideo_MiniV1d5_x2.glsl` | `esrgan` | — | — | W2xEX model weights have no verifiable license grant. |
| `ESRGAN/W2xEX_AnimeVideo_MiniV1d7_x2.glsl` | `esrgan` | — | — | W2xEX model weights have no verifiable license grant. |
| `ESRGAN/W2xEX_AnimeVideo_MiniV1d8_x2.glsl` | `esrgan` | — | — | W2xEX model weights have no verifiable license grant. |
| `ESRGAN/W2xEX_AnimeVideo_Mini_x2.glsl` | `esrgan` | — | — | W2xEX model weights have no verifiable license grant. |
| `ESRGAN/W2xEX_Omni_MiniV2_x2.glsl` | `esrgan` | — | — | W2xEX model weights have no verifiable license grant. |
| `ESRGAN/Zarxrax_Anime1080Fixer_SUC_x1.glsl` | `esrgan` | — | — | Zarxrax model weights have no verifiable license grant. |
| `ESRGAN/Zarxrax_DigitalFlim_SUC_x2.glsl` | `esrgan` | — | — | Zarxrax model weights have no verifiable license grant. |
| `ESRGAN/Zarxrax_HurrDeblur_SUC_x1.glsl` | `esrgan` | — | — | Zarxrax model weights have no verifiable license grant. |
| `ESRGAN/k7_modernAnime_DT_FHD_x2.glsl` | `esrgan` | — | — | k7 modernAnime model weight grant and conversion chain could not be proven. |
| `ESRGAN/k7_modernAnime_FHD_x2.glsl` | `esrgan` | — | — | k7 modernAnime model weight grant and conversion chain could not be proven. |
| `ETC/FineDehalo_lite_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/FineSharp_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/TsubaUP.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/darkside_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/denoise_gaussian_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/fast_gaussian_blur_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/harmony_3LP_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/harmony_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/kBFDN_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/kCCD_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/kGrainFactory_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/kantiring_ms_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `ETC/kgrain_ada_RT.glsl` | `etc` | — | — | Utility/effect shader has no explicit license or verified upstream. |
| `LUT/FGCine_freebdl_RT.glsl` | `lut` | — | — | LUT image data has no separate redistribution license; code license alone is insufficient. |
| `LUT/FGCine_freebdl_fin_RT.glsl` | `lut` | — | — | LUT image data has no separate redistribution license; code license alone is insufficient. |
| `LUT/Krell_pinkgirl_RT.glsl` | `lut` | — | — | LUT image data has no separate redistribution license; code license alone is insufficient. |
| `LUT/Krell_pinkgirl_fin_RT.glsl` | `lut` | — | — | LUT image data has no separate redistribution license; code license alone is insufficient. |
| `nlmeans/knlm2_RT.glsl` | `nlmeans` | — | — | KNLM variant's original implementation and license could not be verified. |
| `nlmeans/knlm_lite_RT.glsl` | `nlmeans` | — | — | KNLM variant's original implementation and license could not be verified. |
| `Pixel-art/MMPX.glsl` | `pixelart` | — | — | MMPX license and original repository could not be verified. |
| `Pixel-art/OmniScale_RT.glsl` | `pixelart` | — | — | OmniScale license and original repository could not be verified. |
| `QCOM/QCOM_SGEDS_ms_RT.glsl` | `qcom` | — | — | SGEDS/SGSR conversion chain from Snapdragon GSR is incomplete and unverified. |
| `QCOM/QCOM_SGSR_RT.glsl` | `qcom` | — | — | SGEDS/SGSR conversion chain from Snapdragon GSR is incomplete and unverified. |
| `QCOM/QCOM_SGSR_luma_RT.glsl` | `qcom` | — | — | SGEDS/SGSR conversion chain from Snapdragon GSR is incomplete and unverified. |
| `QCOM/QCOM_quicksrnet_medium_x1p5.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_medium_x2.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_medium_x3.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_medium_x4.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_small_x1p5.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_small_x2.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_small_x3.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/QCOM_quicksrnet_small_x4.glsl` | `qcom` | BSD-3-Clause | BSD-3-Clause | AIMET QuickSRNet weights are BSD-3-Clause, but the hooke007 GLSL conversion and weight hashes cannot be independently verified. Not redistributed. |
| `QCOM/kSGEDS_RT.glsl` | `qcom` | — | — | SGEDS/SGSR conversion chain from Snapdragon GSR is incomplete and unverified. |
| `RAISR/Intel_RAISR_x1p5_denoise.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `RAISR/Intel_RAISR_x1p5_highres.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `RAISR/Intel_RAISR_x2_denoise.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `RAISR/Intel_RAISR_x2_highres.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `RAISR/Intel_RAISR_x2_highres_sharp.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `RAISR/Intel_RAISR_x2_lowres.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `RAISR/Intel_RAISR_x2_lowres_sharp.glsl` | `raisr` | — | — | Intel RAISR shader port has no verified license chain. |
| `USM/LumaSharpen_RT.glsl` | `sharpening` | — | — | Unsharp-mask shader has no verified original license. |
| `USM/minblur_usm.glsl` | `sharpening` | — | — | Unsharp-mask shader has no verified original license. |
| `USM/sharpen_complexV2_RT.glsl` | `sharpening` | — | — | Unsharp-mask shader has no verified original license. |
| `USM/unsharp_RT.glsl` | `sharpening` | — | — | Unsharp-mask shader has no verified original license. |
| `USM/unsharp_masking_RT.glsl` | `sharpening` | — | — | Unsharp-mask shader has no verified original license. |
| `WarpSharp/aWarpSharp2_RT.glsl` | `warpsharp` | GPL-2.0-or-later | — | aWarpSharp2 is a VapourSynth plugin; this GLSL port's license is not established. |
| `WarpSharp/aWarpSharp3_RT.glsl` | `warpsharp` | GPL-2.0-or-later | — | aWarpSharp2 is a VapourSynth plugin; this GLSL port's license is not established. |
| `WarpSharp/kwarpline_RT.glsl` | `warpsharp` | GPL-2.0-or-later | — | aWarpSharp2 is a VapourSynth plugin; this GLSL port's license is not established. |

### SAFE_WITH_NOTICE（53 条）

| 路径 | family | 代码许可证 | 模型许可证 | 处置/原因 |
| --- | --- | --- | --- | --- |
| `ArtCNN/ArtCNN_Legacy_C4F16_DS_YCbCr.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_Legacy_C4F16_RGB.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_Legacy_C4F16_YCbCr.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_Legacy_C4F32_Chroma.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_Legacy_C4F32_DS_YCbCr.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_Legacy_C4F32_RGB.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_Legacy_C4F32_YCbCr.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_v0_C4F8.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_v0_C4F8_CMP.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_v0_C4F8_DS.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `ArtCNN/ArtCNN_v0_C4F8_DS_CMP.glsl` | `artcnn` | MIT | MIT | Older official ArtCNN generation. The current MIT-pinned ArtCNN release ships the C4F16/C4F32 files; this older variant was not selected.；替代来源：ArtCNN C4F16/C4F32 (current upstream) |
| `Deband/hdeband_rgb_RT.glsl` | `deband` | LGPL-2.1-or-later | — | hooke007 RGB wrapper around AN3223 hdeband; the original upstream file is shipped instead.；替代来源：Deband/hdeband.glsl |
| `FSRCNNX/FSRCNNX_x1_16_0_4_1_anime_distort.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `FSRCNNX/FSRCNNX_x1_16_0_4_1_distort.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `FSRCNNX/FSRCNNX_x2_16_0_4_1_anime_distort.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `FSRCNNX/FSRCNNX_x2_16_0_4_1_distort.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `FSRCNNX/FSRCNNX_x2_56_16_4_1.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `FSRCNNX/FSRCNNX_x2_8_0_4_1.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `FSRCNNX/FSRCNNX_x2_8_0_4_1_LineArt.glsl` | `fsrcnnx` | LGPL-3.0-or-later | LGPL-3.0-or-later | Variant carries an explicit LGPL header but is not part of the pinned FSRCNNX release asset; only the official x2 16-0-4-1 asset is shipped.；替代来源：FSRCNNX/FSRCNNX_x2_16_0_4_1.glsl |
| `Pixel-art/hqx_lite_RT.glsl` | `pixelart` | LGPL-2.1-or-later | — | hqx-shader is LGPL-2.1-or-later, but the mpv port embeds LUT data; the reference file was not copied and the original GLSL+LUT source is preferred. |
| `Pixel-art/hqx_x2.glsl` | `pixelart` | LGPL-2.1-or-later | — | hqx-shader is LGPL-2.1-or-later, but the mpv port embeds LUT data; the reference file was not copied and the original GLSL+LUT source is preferred. |
| `Pixel-art/hqx_x3.glsl` | `pixelart` | LGPL-2.1-or-later | — | hqx-shader is LGPL-2.1-or-later, but the mpv port embeds LUT data; the reference file was not copied and the original GLSL+LUT source is preferred. |
| `Pixel-art/hqx_x4.glsl` | `pixelart` | LGPL-2.1-or-later | — | hqx-shader is LGPL-2.1-or-later, but the mpv port embeds LUT data; the reference file was not copied and the original GLSL+LUT source is preferred. |
| `Pixel-art/superxbr.glsl` | `pixelart` | LGPL-3.0-or-later | — | SuperXBR GLSL port carries an LGPL header, but the reference multi-pass retroarch shader was not adapted or copied. |
| `Pixel-art/superxbr_rgb.glsl` | `pixelart` | LGPL-3.0-or-later | — | SuperXBR GLSL port carries an LGPL header, but the reference multi-pass retroarch shader was not adapted or copied. |
| `Pixel-art/superxbr_yuv.glsl` | `pixelart` | LGPL-3.0-or-later | — | SuperXBR GLSL port carries an LGPL header, but the reference multi-pass retroarch shader was not adapted or copied. |
| `RAISR/ravu_3x_r2_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_3x_r2_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_3x_r3_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_3x_r3_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_3x_r4_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_3x_r4_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_r2_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_r2_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_r3_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_r3_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_r4_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_r4_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_ar_r2_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_ar_r2_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_ar_r3_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_ar_r3_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_pc_r3.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_pc_r3_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_r2_chroma.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_r2_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_r2_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_r3_chroma.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_r3_rgb.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `RAISR/ravu_zoom_r3_yuv.glsl` | `raisr` | LGPL-3.0-or-later | LGPL-3.0-or-later | RAVU LUT shader derived from bjin/mpv-prescalers under LGPL-3.0-or-later. |
| `USM/GEGL_unsharp_mask_RT.glsl` | `sharpening` | GPL-3.0-or-later | — | GEGL unsharp-mask code is GPL-3.0-or-later; the reference file only links to the license and was not copied. |
| `USM/GEGL_unsharp_mask_luma_RT.glsl` | `sharpening` | GPL-3.0-or-later | — | GEGL unsharp-mask code is GPL-3.0-or-later; the reference file only links to the license and was not copied. |
| `USM/GEGL_unsharp_mask_scl_RT.glsl` | `sharpening` | GPL-3.0-or-later | — | GEGL unsharp-mask code is GPL-3.0-or-later; the reference file only links to the license and was not copied. |

## 字段定义

`manifests/shader-audit.json` 的每个条目包含：

- `hooke_path`
- `family`
- `original_upstream`
- `upstream_path`
- `same_or_modified`
- `code_license`
- `model_license`
- `status`
- `reason`
- `replacement_source`
- `included_in_tls`
- `upstream_commit`

纳入条目还包含 `tls_target` 与 `tls_sha256`，必须与 `manifests/shaders.yml`
当前声明的目标和 SHA-256 完全一致；目标集合必须与 `build.shaders.shader_targets()`
完全相等。状态定义：

- `SAFE`：代码和数据/权重许可证链完整且允许再分发。
- `SAFE_WITH_NOTICE`：允许再分发，但需要保留额外通知、派生声明或版权说明。
- `NONCOMMERCIAL`：至少一个组成要素禁止商业使用，不得随包发布。
- `NO_REDISTRIBUTION`：明确禁止再分发。
- `UNKNOWN`：来源、许可证或权重授权无法证明；默认排除。
- `CLEAN_ROOM_REIMPLEMENT_CANDIDATE`：功能可保留，但现有文件没有可证明的
  许可证链，只允许原创 clean-room 实现，不得复制原文件。

## 更新与复核

`manifests/shaders.yml` 的 `update.mode` 控制月度自动刷新：

- `latest-commit`：跟随上游默认分支 HEAD，版本写为提交日期。
- `latest-tag`：跟随最新非 prerelease Release tag。
- `pinned`：不自动移动（GAN 旧分支、GPL/特殊释放资产等）。

`python -m build.shader_update` 只刷新已声明文件；任何文件缺失、许可证文本
变化或 `license_marker` 消失都会失败并要求人工复核。刷新后的 manifest 还必须
通过 `build.resolve`、双平台构建和 `build.verify` 才能进入发布。

