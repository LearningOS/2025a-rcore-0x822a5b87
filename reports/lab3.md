# lab3

在本章节中，我们在shell进程下实现了：
1. `sys_mmap`， `sys_munmap`， `sys_get_time` 等系统调用；
2. `sys_spawn` 创建进程并执行；
3. stride 调度算法；

实现中：

1. 共享页面的 COW；
2. 多种调度算法；
