// 封装返回结果
#[repr(i32)]
#[derive(Copy, Clone)]
pub enum Code {
    // 成功：服务器成功接收客户端请求
    Ok = 200,

    // 未认证：客户端未通过身份验证
    Unauthorized = 401,

    // 禁止访问：客户端没有访问内容的权限
    Forbidden = 403,

    // 未找到：服务器无法找到请求的资源
    NotFound = 404,

    // 请求过多：流量控制限制
    MethodNotAllowed = 405,

    // 请求过多：流量控制限制
    TooManyRequests = 429,

    // 身份验证错误：Token 或 AppKey 已过期
    IdentifyError = 430,

    // 身份验证过期：认证信息已过期
    IdentifyExpired = 431,

    // 签名错误：请求签名验证失败
    SignError = 432,

    // 服务器错误：服务器遇到错误，无法完成请求
    InternalServerError = 500,

    // 文件过大：超出最大允许上传文件大小
    FileTooLarge = 800,

    // 缺少必要请求头：请求中缺少必要头部字段
    MissingHeader = 900,

    // 参数缺少：缺少必要参数
    MissingParam = 901,

    // 参数不合法：客户端请求包含非法参数
    IllegalParam = 902,

    // 校验相关错误码
    // 字段必填
    ValidationRequired = 1001,
    // 长度必须在范围内
    ValidationLengthBetween = 1002,
    // 长度必须至少为
    ValidationLengthMin = 1003,
    // 长度必须至多为
    ValidationLengthMax = 1004,
    // 长度无效
    ValidationLengthInvalid = 1005,
    // 数值必须在范围内
    ValidationRangeBetween = 1006,
    // 数值必须至少为
    ValidationRangeMin = 1007,
    // 数值必须至多为
    ValidationRangeMax = 1008,
    // 数值超出范围
    ValidationRangeInvalid = 1009,
    // 必须是有效的电子邮件地址
    ValidationEmail = 1010,
    // 无效（未知的校验错误）
    ValidationUnknown = 1011,

    // 沙箱相关错误
    // 创建沙箱失败
    SandboxCreateError = 2001,
    // 镜像拉取失败
    ImagePullError = 2002,
    // 镜像不存在
    ImageNotFound = 2003,
    // 连接 Docker 失败
    DockerConnectError = 2004,
    // 删除沙箱失败
    SandboxDeleteError = 2005,
    // 沙箱不存在
    SandboxNotFound = 2006,
    // 获取沙箱详情失败
    SandboxGetError = 2007,
    // 沙箱生命周期操作失败
    SandboxLifecycleError = 2008,
    // 续期参数无效
    InvalidRenewExpiration = 2011,
}

impl From<Code> for i32 {
    fn from(code: Code) -> Self {
        code as i32
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", i32::from(*self))
    }
}

#[test]
fn test_code() {
    assert_eq!(Code::Ok as i32, 200);
    assert_eq!(Code::Ok.to_string(), "200");
    assert_eq!(format!("{}", Code::InternalServerError), "500");
}
