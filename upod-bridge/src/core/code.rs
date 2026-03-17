// 封装返回结果
#[repr(i32)]
#[derive(Copy, Clone)]
pub enum Code {
    // 成功：服务器成功接收客户端请求
    Ok = 200,
}

impl From<Code> for i32 {
    fn from(code: Code) -> Self {
        code as i32
    }
}
