// 引入依赖库
use inquire::{
    validator::Validation,
    Text,
};
use fancy_regex::Regex;
use reqwest::{blocking as req, redirect::Policy, header};
use serde::Deserialize;
use serde_json;
use colored::*;

// 常量部分，主要用于正则表达式匹配和B站API
const REG_BVID: &str = r"BV\w{10}";
const REG_AVID: &str = r"av\d{1,9}";
// const REG_URL: &str = r"(.*)bilibili.com/video/(BV\w{10}|av\d{1,9})";
const REG_URL: &str = r"(.*)bilibili.com/video/(BV\w{10}|av\d{1,9})(?=/|\?|$)";
const REG_SHORT_URL: &str = r"(http(s|)://|^)b23.tv/(\w+)";
const API_VIDEO_INFO: &str = "https://api.bilibili.com/x/web-interface/view";
const API_STREAM_URL: &str = "https://api.bilibili.com/x/player/wbi/playurl";
const API_USER_INFO: &str = "https://api.bilibili.com/x/web-interface/nav";
const HTTP_REFERER: &str = "www.bilibili.com";

enum VideoIdValue {
    Avid(u32),
    Bvid(String),
}

struct VideoId {
    value: VideoIdValue,
}

impl VideoId {
    fn get_key(&self) -> &str {
        match self.value {
            VideoIdValue::Avid(_) => "aid",
            VideoIdValue::Bvid(_) => "bvid"
        }
    }
    fn to_string(&self) -> String {
        match &self.value {
            VideoIdValue::Avid(t) => t.to_string(),
            VideoIdValue::Bvid(t) => t.clone(),
        }
    }
    fn new(av_or_bvid: &str) -> Result<Self, String> {
        match Regex::new(REG_AVID).unwrap().captures(av_or_bvid).unwrap() {
            Some(t) => match t[0][2..].parse::<u32>() {
                Ok(t) => Ok(Self { value: VideoIdValue::Avid(t) }),
                Err(t) => Err(t.to_string())
            },
            None => match Regex::new(REG_BVID).unwrap().captures(av_or_bvid).unwrap() {
                Some(t) => Ok(Self { value: VideoIdValue::Bvid(t[0].to_string()) }),
                None => Err("在输入的字符串中未找到有效的av/bvid".into())
            }
        }
    }
}

struct PageInfo {
    cid: u32,
    p: u32,
    title: String,
}

struct VideoInfo {
    bvid: String,
    title: String,
    uploader: String,
    pages: Vec<PageInfo>,
}

enum UserState {
    Vip(String),
    User(String),
    None,
}

struct UserInfo {
    state: UserState,
    img_url: String,
    sub_url: String,
}

// 主函数，主要处理用户输入和程序整体流程
fn main() {
    // inquire预验证规则，只按照正则表达式进行匹配判断
    let validator = |input: &str| {
        if Regex::new(REG_AVID).unwrap().is_match(input).unwrap() ||
            Regex::new(REG_BVID).unwrap().is_match(input).unwrap() ||
            Regex::new(REG_URL).unwrap().is_match(input).unwrap() ||
            Regex::new(REG_SHORT_URL).unwrap().is_match(input).unwrap() {
            Ok(Validation::Valid)
        } else {
            Ok(Validation::Invalid("请输入正确的视频链接或BV/av号".into()))
        }
    };

    // 简单提取常规长url中的av/bv号，用于inquire回显展示，防止长链接换行不美观（av/bv号输入和短链接输入不管）
    let format_to_id = &|input: &_| {
        if Regex::new(REG_URL).unwrap().is_match(input).unwrap() {
            match Regex::new(REG_BVID).unwrap().captures(input).unwrap() {
                Some(caps) => caps[0].to_string(),
                None => Regex::new(REG_AVID).unwrap()
                    .captures(input).unwrap().unwrap()[0].to_string(),
            }
        } else {
            input.to_string()
        }
    };

    // 构造inquire请求对象
    let video_inquirer = Text::new("请输入要下载的视频链接或BV/av号")
        .with_help_message("B站视频链接、b23.tv短连接、BV号或av号均可")
        .with_validator(validator)
        .with_formatter(format_to_id);

    let mut input_invalid = true;
    let mut video_id = VideoId::new("av0").unwrap();
    let mut video_info = VideoInfo {
        bvid: "".into(),
        title: "".into(),
        uploader: "".into(),
        pages: Vec::new(),
    };

    let cookie = Text::new("请输入Cookie SESSDATA =").prompt().unwrap();
    let mut headers = header::HeaderMap::new();
    headers.insert(header::COOKIE, header::HeaderValue::from_str(&*format!("SESSDATA={}", &cookie)).unwrap());
    let client = req::Client::builder()
        .default_headers(headers)
        .build().unwrap();

    // 验证Cookie有效性及获取用户信息
    let user_info = get_user_info(&client).unwrap();
    match user_info.state {
        UserState::None => println!("{}", "Cookie无效，未登录状态".yellow()),
        UserState::User(t) => println!("{}", format!("普通用户：{}，你好~", t).green()),
        UserState::Vip(t) => println!("{}", format!("大会员用户：{}，你好~", t).truecolor(251, 114, 153))
    }

    // 询问+处理逻辑，当处理出错时（短链接404、长链接格式有误等正则查不出来等错误）循环提示用户重新输入
    while input_invalid {
        let res = video_inquirer.clone().prompt().unwrap();
        match parse_video_id(&res) {
            Ok(t) => {
                video_id = t;
                input_invalid = false;
            }
            Err(e) => println!("{}", e.bold().red())
        };
        match get_video_info(&video_id, &client) {
            Ok(t) => video_info = t,
            Err(e) => {
                println!("{}", e.bold().red());
                input_invalid = true;
            }
        }
    }

    println!("1")
}

// 将用户输入的视频url、短链接、av/bv号等统一处理成av/bv号，方便后续请求
fn parse_video_id(input: &str) -> Result<VideoId, String> {
    let reg_bvid = Regex::new(REG_BVID).unwrap();
    let reg_avid = Regex::new(REG_AVID).unwrap();
    let reg_url = Regex::new(REG_URL).unwrap();
    let reg_short_url = Regex::new(REG_SHORT_URL).unwrap();
    let url_to_id = |a: &str| -> Result<VideoId, String> {
        let processed_url = match reg_url.captures(a).unwrap() {
            Some(t) => t[0].to_string(),
            None => return Err("解析视频Url出错".into())
        };
        VideoId::new(&processed_url)
    };
    if reg_url.is_match(input).unwrap() {
        url_to_id(input)
    } else if reg_bvid.is_match(input).unwrap() || reg_avid.is_match(input).unwrap() {
        VideoId::new(input)
    } else if reg_short_url.is_match(input).unwrap() {
        let processed_short_url = match reg_short_url.captures(input).unwrap() {
            Some(t) => t[0].to_string(),
            None => return Err("解析短Url出错".into())
        };
        match parse_short_url(&processed_short_url) {
            Some(t) => url_to_id(&t),
            None => Err("该b23.tv短链接无效".into())
        }
    } else {
        Err("视频链接无效".into())
    }
}

// 解析b23.tv短链接
fn parse_short_url(short_url: &str) -> Option<String> {
    let client = req::Client::builder()
        .redirect(Policy::none())
        .build()
        .unwrap();
    let resp = match client.get(short_url).send() {
        Ok(t) => t,
        Err(..) => return None
    };
    if resp.status().is_redirection() {
        match resp.headers().get("Location") {
            Some(t) => Some(t.to_str().unwrap().to_string()),
            None => None
        }
    } else {
        None
    }
}

// 校验Cookie是否有效
fn get_user_info(client: &req::Client) -> Result<UserInfo, String> {
    #[derive(Deserialize)]
    struct WbiImg {
        img_url: String,
        sub_url: String,
    }
    #[derive(Deserialize)]
    struct RawData {
        isLogin: bool,
        wbi_img: WbiImg,
    }
    #[derive(Deserialize)]
    struct RawResponse {
        code: i32,
        data: RawData,
    }
    let res = client.get(API_USER_INFO).send();
    let res = match res {
        Ok(t) => t,
        Err(_) => return Err("网络错误".into())
    };
    let res = match res.text() {
        Ok(t) => t,
        Err(_) => return Err("响应异常".into())
    };
    let pre_res: RawResponse = match serde_json::from_str(&res) {
        Ok(t) => t,
        Err(_) => return Err("响应异常".into())
    };
    if pre_res.code == -101 && !pre_res.data.isLogin {
        return Ok(UserInfo {
            state: UserState::None,
            img_url: pre_res.data.wbi_img.img_url,
            sub_url: pre_res.data.wbi_img.sub_url,
        });
    } else if pre_res.code != 0 || !pre_res.data.isLogin { return Err("无法理解的响应".into()); };
    #[derive(Deserialize)]
    struct PostRawData {
        uname: String,
        vipStatus: u8,
    }
    #[derive(Deserialize)]
    struct PostRawResponse {
        data: PostRawData,
    }
    let post_res: PostRawResponse = match serde_json::from_str(&res) {
        Ok(t) => t,
        Err(_) => return Err("响应异常".into())
    };
    if post_res.data.vipStatus == 0 {
        Ok(UserInfo {
            state: UserState::User(post_res.data.uname),
            img_url: pre_res.data.wbi_img.img_url,
            sub_url: pre_res.data.wbi_img.sub_url,
        })
    } else if post_res.data.vipStatus == 1 {
        Ok(UserInfo {
            state: UserState::Vip(post_res.data.uname),
            img_url: pre_res.data.wbi_img.img_url,
            sub_url: pre_res.data.wbi_img.sub_url,
        })
    } else { Err("用户已登录，但存在无法理解的响应".into()) }
}

// 获取视频信息，也用于预检视频是否有效
fn get_video_info(video_id: &VideoId, client: &req::Client) -> Result<VideoInfo, String> {
    #[derive(Deserialize)]
    struct RawOwner {
        name: String,
    }
    #[derive(Deserialize)]
    struct RawPage {
        cid: u32,
        page: u32,
        part: String,
    }
    #[derive(Deserialize)]
    struct RawInfo {
        bvid: String,
        title: String,
        owner: RawOwner,
        pages: Vec<RawPage>,
    }
    #[derive(Deserialize)]
    struct RawResponse {
        code: i32,
        data: RawInfo,
    }
    let res = client.get(API_VIDEO_INFO)
        .query(&[(video_id.get_key(), video_id.to_string())])
        .send();
    let res = match res {
        Ok(t) => t,
        Err(_) => return Err("网络错误".into())
    };
    let res: RawResponse = match res.json() {
        Ok(t) => t,
        Err(_) => return Err("响应异常，视频可能不存在".into())
    };
    if res.code != 0 { return Err(format!("视频状态异常：{}", res.code)); }
    let mut pages: Vec<PageInfo> = Vec::new();
    for i in res.data.pages.iter() {
        pages.push(PageInfo {
            title: String::from(&i.part),
            cid: i.cid,
            p: i.page,
        })
    }
    Ok(VideoInfo {
        bvid: res.data.bvid,
        title: res.data.title,
        uploader: res.data.owner.name,
        pages,
    })
}