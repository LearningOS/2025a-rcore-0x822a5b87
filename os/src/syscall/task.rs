use crate::batch;
use crate::batch::Task;

pub fn sys_get_task_info() -> isize {
    let mut task = Task{
        num_app: [0; 5],
        app_start: 0,
        app_end: 0,
    };
    println!("=========================sys_get_task_info start=========================");
    // object -> reference -> raw pointer -> usize
    let task_ptr: *mut Task = &mut task;
    println!("sys_get_task_info task ptr = {:?}", task_ptr);
    let task : Task = batch::get_task_info();
    println!("sys_get_task_info task = {:?}", task);
    unsafe {
        (*task_ptr).num_app = task.num_app;
        (*task_ptr).app_start = task.app_start;
        (*task_ptr).app_end = task.app_end;
    }
    println!("=========================sys_get_task_info end=========================");
    task_ptr as isize
}
