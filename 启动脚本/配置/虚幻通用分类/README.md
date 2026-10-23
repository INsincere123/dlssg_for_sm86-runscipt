# RTX 30 系 UE4 / UE5 四档通用性能候选

本目录共八份独立 Engine.ini：UE4、UE5 各四档。日期：2026-10-24；已完成第二参考筛选修订，见 [第二参考优化说明](第二参考优化说明.md)。它们是从参考项目筛选、结合个人偏好重新设计的候选，不是把上游整份配置按比例缩小，也不是已验证的最佳配置。

不改动已有鸣潮、异环、SteamUE5 默认源、游戏原件、启动器预设或导出清单；不启动游戏、不打包、不自动拉取上游。图形启动器已接入UE主版本识别、手动校准和画质选项；不根据显卡自动选档。

## 选择档位

| 档位 | 定位与取舍 | 桌面端初步参考 | 笔记本初步参考 | 输出建议（不写进INI） |
|---|---|---|---|---|
| 01-高性能均衡 | 参考3080 Ti Laptop 16GB偏好；保留主要反射、GI与近处阴影精度，小幅收缩远景 | 3070/3070 Ti/3080 10或12GB/3080 Ti/3090/3090 Ti | 3080 Ti及性能接近的高功耗3080 | 1440p～4K，按游戏选择超分 |
| 02-主流均衡 | 减少远景、阴影、雾及反射开销，接受中等画质取舍 | 3060 Ti（含显存类型变体） | 3070/3070 Ti，部分低功耗3080/3080 Ti | 偏向1440p |
| 03-性能优先 | 更积极简化距离、阴影与间接光空间细节 | 3060 8GB/12GB | 3060及部分低功耗3070 | 偏向1080p |
| 04-入门流畅 | 更明显的阴影、雾、景深及光照细节取舍，仍不强关核心光照或Nanite | 3050 6GB/8GB | 3050 4GB/6GB、3050 Ti | 偏向1080p，必要时调整游戏内超分 |

以上是配置设计建议，不是统一基准测得的等性能关系。笔记本功耗、散热、独显直连、CPU与供电会造成跨档重叠；3090等更强硬件用第一档只会留更多余量，不为用满显卡额外增加负担。OEM、特殊版本和低功耗机型按实际表现校正；不声称覆盖每一个型号/功耗组合的实测。

性能档与显存预算分开：3060 12GB不能仅因显存大就升到第一档；3070 8GB也不适合照搬个人16GB机器的10240MB纹理池。八份INI均不指定固定纹理/Nanite池，不强制全mip常驻。低显存设备优先在游戏菜单降低纹理需求；扩大池不能保证解决共享显存或卡顿。降低各向异性过滤主要影响采样带宽与斜面清晰度，不是降低纹理显存占用的替代措施。

## 通用参数矩阵

“沿用”表示不写覆盖，实际游戏值未知。以下绝对值可能高于游戏现值，不保证成本严格逐档下降；只有同场景验证才能判断。

| 参数 | 第一档 | 第二档 | 第三档 | 第四档 |
|---|---|---|---|---|
| `r.TextureStreaming` | 1 | 1 | 1 | 1 |
| `r.Streaming.DefragDynamicBounds` | 1 | 1 | 1 | 1 |
| `r.Streaming.LimitPoolSizeToVRAM` | 1 | 1 | 1 | 1 |
| `r.Streaming.FramesForFullUpdate` | 沿用游戏 | 沿用游戏 | 沿用游戏 | 沿用游戏 |
| `r.Streaming.FullyLoadUsedTextures` | 0 | 0 | 0 | 0 |
| `r.Streaming.UsePerTextureBias` | 1 | 1 | 1 | 1 |
| `a.URO.Enable` | 1 | 1 | 1 | 1 |
| `r.ViewDistanceScale` | 0.85 | 0.75 | 0.65 | 0.55 |
| `r.StaticMeshLODDistanceScale` | 1.1 | 1.25 | 1.5 | 1.75 |
| `foliage.LODDistanceScale` | 0.9 | 0.8 | 0.65 | 0.5 |
| `grass.CullDistanceScale` | 0.9 | 0.8 | 0.65 | 0.5 |
| `r.Shadow.DistanceScale` | 0.85 | 0.75 | 0.65 | 0.5 |
| `r.Shadow.MaxResolution` | 沿用游戏 | 2048 | 1024 | 512 |
| `r.Shadow.CSM.MaxCascades` | 沿用游戏 | 3 | 2 | 1 |
| `r.MaxAnisotropy` | 16 | 16 | 8 | 4 |
| `r.SceneColorFringeQuality` | 0 | 0 | 0 | 0 |
| `r.DepthOfFieldQuality` | 沿用游戏 | 1 | 1 | 0 |
| `r.BloomQuality` | 沿用游戏 | 4 | 4 | 3 |
| `r.LensFlareQuality` | 沿用游戏 | 1 | 1 | 0 |
| `r.VolumetricFog.GridPixelSize` | 沿用游戏 | 12 | 16 | 24 |
| `r.VolumetricFog.GridSizeZ` | 沿用游戏 | 96 | 64 | 48 |

第一档不限制传统阴影分辨率与级联、不覆盖景深/泛光/雾网格，尽量贴近现有偏好；第二档开始限制这些项目。前三档并不是对角色等指定对象的豁免：全局参数仍可能影响近景。无粒子数量、特效生成率和DetailMode覆盖，避免主动改变战斗提示或游戏逻辑。

纹理流送更新间隔不再统一固定为10，也不照搬第二参考的2：这是CPU开销与贴图响应的取舍，不能仅按显卡型号决定。新增FullyLoadUsedTextures=0避免最高mip强制常驻；UsePerTextureBias=1只改变已有MipBias的应用方式，未写全局MipBias，不保证自动降低显存或提升帧率。

## UE4 与 UE5 的差别

UE4 以4.27常见PC路径作为参考，借鉴鸣潮通用流送与URO思路，不包含库洛/HT/WorldPartition、项目资源Paths，也不移植鸣潮固定纹理池。第一档保持游戏反射路径；第二至四档只缩减已有SSR，不强制开SSR或关闭游戏RT。不包含Lumen、Nanite、TSR、VSM或MegaLights键。

| UE4专用参数 | 第一档 | 第二档 | 第三档 | 第四档 |
|---|---|---|---|---|
| `r.SSR.Quality` | 沿用游戏 | 2 | 2 | 1 |
| `r.SSR.HalfResSceneColor` | 沿用游戏 | 1 | 1 | 1 |

UE5 以5.6资料及现有异环偏好作为参数参考。UE5早期或定制分支可能缺少部分CVar，不保证所有UE5小版本均支持。第一档保留反射分辨率、GI屏幕探针间距与VSM采样的游戏选择，采用现有偏好的粗糙度阈值及探针缓存分辨率；后续降低空间精度与阴影采样。所有档都保留游戏的GI/反射方法、时间滤波、屏幕追踪和更新预算，不强制关闭Lumen、Nanite或硬件光追。游戏本来使用硬件光追时，低档仍可能很昂贵，应优先在游戏菜单选择已验证的低成本路径。

| UE5专用参数 | 第一档 | 第二档 | 第三档 | 第四档 |
|---|---|---|---|---|
| `r.Lumen.Reflections.MaxRoughnessToTraceClamp` | 0.3 | 0.25 | 0.2 | 0.15 |
| `r.Lumen.Reflections.MaxRoughnessToTraceForFoliage` | 0.2 | 0.15 | 0.1 | 0.05 |
| `r.Lumen.ScreenProbeGather.RadianceCache.ProbeResolution` | 16 | 16 | 8 | 8 |
| `r.Lumen.ScreenProbeGather.DownsampleFactor` | 沿用游戏 | 24 | 32 | 32 |
| `r.Lumen.Reflections.DownsampleFactor` | 沿用游戏 | 2 | 2 | 2 |
| `r.Shadow.Virtual.ResolutionLodBiasLocal` | 沿用游戏 | 1 | 1.5 | 2 |
| `r.Shadow.Virtual.ResolutionLodBiasLocalMoving` | 沿用游戏 | 1.5 | 2 | 2.5 |
| `r.Shadow.Virtual.SMRT.SamplesPerRayDirectional` | 沿用游戏 | 3 | 2 | 1 |
| `r.Shadow.Virtual.SMRT.SamplesPerRayLocal` | 沿用游戏 | 3 | 2 | 1 |
| `r.Nanite.MaxPixelsPerEdge` | 沿用游戏 | 1.5 | 2 | 3 |

粗糙度阈值、探针8、间距24及部分VSM采样/LOD数值是本轮自定候选，不是Epic对30系的官方推荐。缩减粗糙表面/植被反射也可能影响水面、车漆与皮肤，并不按物体名称豁免。降低探针分辨率、雾网格及阴影采样可能带来噪点、漏光、闪烁或条带；优先单组回退。静态网格LOD与Nanite是不同路径；第二参考修订为UE5第二至四档增加Nanite像素边长目标1.5/2/3，值越大几何越粗。这是自定测试候选，不是30系官方推荐，可能简化轮廓、植被或造成运动跳变；第一档不覆盖，不切换回退模型。

## 使用、检查与回退

1. 确认游戏是对应UE主版本，先原版启动并备份原Engine.ini，保存SHA256。
2. 选择一份完整候选，通过现有入口的 `-EngineConfigSource` 指定绝对路径；不要与主题片段叠加，不直接覆盖正式默认源。图形入口可在高级设置选择此文件，但本轮未改默认预设。
3. 关闭游戏，完成恢复后再切档。保持输出分辨率、超分、RT、天气、路线和供电条件一致，对原配置及候选各重复至少三轮。
4. 分别记录基础渲染FPS、帧时间尖峰/1%低帧、专用/共享显存与系统内存；插帧后显示FPS单独记。没有超过重复测试波动，不标记有效优化；不承诺基础60或显示120 FPS。
5. 对照人物近影、皮肤、镜面/水面、室内GI、转镜头植被与跑图纹理。异常时回退对应参数组；完整回退使用原字节备份，不写回猜测的默认值。

SystemSettings覆盖可能使游戏菜单无法再覆盖同一CVar；本轮只避免写分辨率、DLSS、同步等键，并不保证其余菜单可覆盖。SSR/Lumen/VSM等参数只在实际启用对应渲染路径时有意义。图形启动器根据检测或手动校准的UE主版本选择对应档位；目录名本身不作为识别证据。

## 来源和明确未采用的内容

- [第二参考本地原件](../虚幻参考/第二参考)、[逐项取舍和优化前备份](第二参考优化说明.md)：四份UET变体仅作参数参考，不复制全量设置。

- [上游参考README](../虚幻参考/UE4and5CommOpti/README.md)，v1.7.1；本轮实际本地提交与来源文件SHA256见 [来源快照.json](来源快照.json)。上游及个人源文件均保留原字节；快照保存哈希，不复制或重置原源。
- [个人UE4参考](../鸣潮/Engine.ini)、[个人UE5参考](../异环/Engine.ini)、[UE5偏好说明](../异环/偏好优化说明.md)。
- [Epic UE4.27伸缩参考](https://dev.epicgames.com/documentation/en-us/unreal-engine/scalability-reference?application_version=4.27)：传统阴影、距离和纹理伸缩思路，本文数值不等于官方整档设置。
- [Epic UE5.6 Lumen性能指南](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine?application_version=5.6)：缓存探针与GI/反射空间分辨率的参考，不把主机目标FPS当成PC承诺。
- [NVIDIA 30系笔记本规格](https://www.nvidia.com/en-ph/geforce/laptops/30-series/)、[Notebookcheck笔记本功耗实测](https://www.notebookcheck.net/Performance-Test-GeForce-RTX-3070-Laptop-RTX-3080-Laptop.516953.0.html)、[Tom's Hardware桌面基准](https://www.tomshardware.com/reviews/gpu-hierarchy,4388.html)：用于初步硬件分组；没有将不同测试平台分数混算为本项目实测收益。

未采用上游关闭日志/崩溃报告/遥测、固定DLSS和AA、关闭同步、全局关RT/静态光照、禁用反射时间滤波/屏幕追踪，以及实验异步与MegaLights开关。不复制上游r.Streaming.PoolSize示例，不把草木/光照质量降档当作免费优化。

## 本轮验证状态

仅完成静态结构与范围检查：UTF-8 BOM、合法节与唯一键、八份文件数量、显式分档值方向、UE4/UE5参数隔离、无强制分辨率/DLSS/固定池/项目Paths和来源哈希不变。八份均是待现场验证的候选；未进行八组实机、跨TGP、跨显存容量或不同引擎小版本验收，也未生成发布包。
