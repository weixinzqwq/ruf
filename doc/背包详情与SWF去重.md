# 背包详情与SWF去重

## 操作与实现

此前完整tick计时定位到sockets阶段的`PetDataService/onViewSinglePet`及嵌套详情派发，再追踪消费者的重复详情读取。

针对2026-11-06核对的官方版本，在`PetCardList.onPetDetailedCached`和`GodCardList.onPetDetailedCached`各删除一次重复`getPetDetailInfo`。保留每个消费者自己的详情对象、事件顺序与资源，不引入全局共享详情对象缓存。

静态检查：152个脚本只有目标两处行为变化；3357个SWF tag仅一个DoABC变化，非代码资源、SymbolClass和时间轴保持。正确候选是历史`newpetpackage-detail-v4.swf`；早期候选已拒用，不能重新交付。

## 版本与测试结论

- 当时`pet/newpetpackage`版本为`2026102936286521`，**不是对2026-11-21官方当前版本的确认**。
- 运行日志已确认实际替换命中；仅有`petPatchMode`不能证明加载成功。
- 每次详情响应少2次读取/解析，但两组显示FPS约11.92/11.94，未见整体改善；窗口像素尺寸和后续错误存在差异。
- 用户当时确认两组显示正常。没有把结果扩大为全场景兼容性或长期性能通过。

## 后续方法

暂不推广这个历史SWF补丁。当前子树A/B入口都使用官方资源，不开启背包本地替换。

若重新做此方向：先核对官方版本与资源哈希，再定位调用链；仅改一项，静态核对脚本、签名、非代码tag与资源，最后检查`startup`里的`pet-swf-replacement`实际命中并做固定窗口/顺序的在线对照。不能直接放宽补丁到任意版本。

卡片数量统计与材料集合遍历保留为后续方向。`getSamplePetCardNumById`在9次详情响应中累计自身耗时约453ms，另有材料属性和Array.filter成本；不要与goto阶段重叠计时相加。

源码入口：`src/optimization/engine/frontend-utils/src/backends/navigator/aola_pet_patch.rs`、`core/src/pet_detail_profile.rs`、`core/src/avm2/function.rs`。原始SWF参考位于`resources/swf-reference`；修改前后快照及重建脚本在[历史修改证据](archive/历史修改证据.zip)的`evidence/pet-detail-20261106`中。

结果：[A/B分析](evidence/history/ab-results-20261106/)、[详情补丁交付记录](evidence/history/pet-detail-20261106/delivery.json)。旧分发包已清理，分析记录和资源原件保留。
