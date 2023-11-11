/*
 rust-bilidown
 个人第一个入门Rust的练手项目，一个B站下载器，基于CLI，支持下载大会员专属格式、支持分P视频、支持选择清晰度
 参考：https://github.com/SocialSisterYi/bilibili-API-collect
 感谢做B站API逆向的易姐以及上述仓库的所有贡献者！
*/

// 引入依赖库
use inquire::{
    validator::Validation,
    Text,
    MultiSelect,
    Select,
    list_option::ListOption,
};
use fancy_regex::Regex;
use reqwest::{blocking as req, redirect::Policy, header};
use serde::Deserialize;
use serde_json as json;
use colored::*;
use std::fmt;
use std::ops::Add;
use std::time::SystemTime;
use urlencoding::encode as url_encode;
use md5;

// 常量部分，主要用于正则表达式匹配和B站API
const REG_BVID: &str = r"BV\w{10}";
const REG_AVID: &str = r"av\d{1,9}";
// const REG_URL: &str = r"(.*)bilibili.com/video/(BV\w{10}|av\d{1,9})";
const REG_URL: &str = r"(.*)bilibili.com/video/(BV\w{10}|av\d{1,9})(?=/|\?|$)";
const REG_SHORT_URL: &str = r"(http(s|)://|^)b23.tv/(\w+)";
const REG_WBI_KEY: &str = r"(?<=i0.hdslb.com/bfs/wbi/)(\w+)(?=\.png)";
const API_VIDEO_INFO: &str = "https://api.bilibili.com/x/web-interface/view";
const API_STREAM_URL: &str = "https://api.bilibili.com/x/player/wbi/playurl";
const API_USER_INFO: &str = "https://api.bilibili.com/x/web-interface/nav";
const HTTP_REFERER: &str = "https://www.bilibili.com";
const HTTP_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15";
const WBI_KEY_TAB: [u8; 64] = [
    46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49, 33, 9,
    42, 19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40, 61, 26, 17, 0,
    1, 60, 51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11, 36, 20, 34, 44, 52
];

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

impl fmt::Display for PageInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "P{}: {}", self.p, self.title)
    }
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
        .user_agent(HTTP_USER_AGENT)
        .default_headers(headers)
        .build().unwrap();

    // 验证Cookie有效性及获取用户信息
    let user_info = get_user_info(&client).unwrap();
    match user_info.state {
        UserState::None => println!("{}", "Cookie无效，未登录状态".yellow()),
        UserState::User(ref t) => println!("{}", format!("普通用户：{}，你好~", t).green()),
        UserState::Vip(ref t) => println!("{}", format!("大会员用户：{}，你好~", t).truecolor(251, 114, 153))
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

    // 展示视频标题、up主基本信息
    println!("{}，UP主 {}", video_info.title, video_info.uploader);

    // 判断视频是否有分p，如有，要求用户选择需要下载的分p，支持多选，之后将修改覆盖到video_info里
    if video_info.pages.len() == 1 {
        println!("该视频无分P，直接下载 {}", video_info.pages[0].title);
    } else if video_info.pages.len() >= 2 {
        let validator = |input: &[ListOption<&PageInfo>]| {
            if input.len() == 0 {
                Ok(Validation::Invalid("至少得选一个视频才能下载啊".into()))
            } else {
                Ok(Validation::Valid)
            }
        };
        let res = MultiSelect::new("选择想下载的分集", video_info.pages)
            .with_validator(validator)
            .prompt().unwrap();
        video_info.pages = res;
    }

    let a = get_stream_url(&video_info.bvid, &video_info.pages[0].cid, true, &user_info, &client).unwrap();

    println!()
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

// 计算B站Wbi签名，见 https://socialsisteryi.github.io/bilibili-API-collect/docs/misc/sign/wbi.html
fn wbi_sign_para(mut paras: Vec<(String, String)>, img_url: &str, sub_url: &str) -> Result<Vec<(String, String)>, String> {
    let reg_wbi = Regex::new(REG_WBI_KEY).unwrap();
    let img_key = match reg_wbi.captures(img_url).unwrap() {
        Some(t) => t[0].to_string(),
        None => return Err("参数不合法".into())
    };
    let sub_key = match reg_wbi.captures(sub_url).unwrap() {
        Some(t) => t[0].to_string(),
        None => return Err("参数不合法".into())
    };
    let key = format!("{}{}", img_key, sub_key);
    let mut mixin_key = String::new();
    let mut buffer = [0; 4];
    for i in WBI_KEY_TAB.iter() {
        mixin_key = mixin_key.add(key.chars().nth(*i as usize).unwrap().encode_utf8(&mut buffer))
    };
    let mixin_key = &mixin_key[0..32];
    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    paras.push(("wts".into(), now.to_string()));
    paras.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut query = Vec::new();
    let reg_filter = Regex::new(r"[!'()*]").unwrap();
    for i in paras.iter() {
        let val = reg_filter.replace(&i.1, "").to_string();
        let key = url_encode(&i.0).to_string().to_lowercase();
        let val = url_encode(&val).to_string().to_lowercase();
        query.push(format!("{}={}", key, val));
    };
    let query = query.join("&");
    let wbi_sign = format!("{:x}", md5::compute(query.add(&mixin_key)));
    paras.push(("w_rid".into(), wbi_sign));
    Ok(paras)
}

// 获取视频流下载链接
fn get_stream_url(bvid: &str, cid: &u32, choose_quality_manually: bool,
                  user_info: &UserInfo, client: &req::Client) -> Result<(String, String), String> {
    let mut quality_flag = match user_info.state {
        UserState::None => vec![("qn".to_string(), "64".to_string()), ("fnval".to_string(), "16".to_string())],
        UserState::User(_) => vec![("qn".to_string(), "80".to_string()), ("fnval".to_string(), "16".to_string())],
        UserState::Vip(_) => vec![("qn".to_string(), "127".to_string()), ("fnval".to_string(), "4048".to_string()),
                                  ("fourk".to_string(), "1".to_string()),
        ]
    };
    let mut paras = vec![("bvid".to_string(), bvid.to_string()),
                         ("cid".to_string(), cid.to_string())];
    paras.append(&mut quality_flag);
    let paras = wbi_sign_para(paras, &user_info.img_url, &user_info.sub_url)?;
    let res = match client.get(API_STREAM_URL).query(&paras).send() {
        Ok(t) => t,
        Err(_) => return Err("网络错误".into())
    };
    let res = res.text().unwrap();
    #[derive(Deserialize)]
    struct RawVideo {
        id: i32,
        base_url: String,
        backup_url: Vec<String>,
        codecid: i32,
    }
    #[derive(Deserialize)]
    struct RawAudio {
        id: i32,
        base_url: String,
        backup_url: Vec<String>,
    }
    #[derive(Deserialize)]
    struct RawDash {
        video: Vec<RawVideo>,
        audio: Vec<RawAudio>,
        dolby: serde_json::Value,
        flac: serde_json::Value,
    }
    #[derive(Deserialize)]
    struct RawData {
        accept_description: Vec<String>,
        accept_quality: Vec<i32>,
        dash: RawDash,
    }
    #[derive(Deserialize)]
    struct RawResponse {
        code: i32,
        data: RawData,
    }
    let mut post_res: RawResponse = match serde_json::from_str(&res) {
        Ok(t) => t,
        Err(a) => return Err("响应异常".into())
    };
    let mut quality_id = post_res.data.accept_quality[0];
    if choose_quality_manually {
        struct Quality(i32, String);
        impl fmt::Display for Quality {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "{}", self.1) }
        }
        let mut qualities: Vec<Quality> = Vec::new();
        for (i, v) in post_res.data.accept_quality.iter().enumerate() {
            qualities.push(Quality(v.clone(), post_res.data.accept_description[i].to_string()))
        }
        let res = Select::new("选择该分P要下载的清晰度", qualities).prompt().unwrap();
        quality_id = res.0
    }
    let mut best_audio = post_res.data.dash.audio.iter().max_by_key(|i| i.id).unwrap().base_url.to_string();
    if let json::Value::Object(t) = post_res.data.dash.flac {
        if let Some(t) = t.get("audio") {
            if let json::Value::Object(t) = t {
                if let Some(t) = t.get("base_url") {
                    if let json::Value::String(t) = t {
                        if !choose_quality_manually {
                            best_audio = t.to_string();
                        }
                    }
                }
            }
        }
    };
    if let json::Value::Object(t) = post_res.data.dash.dolby {
        if let Some(t) = t.get("audio") {
            if let json::Value::Array(t) = t {
                if let Some(t) = t.get(0) {
                    if let json::Value::Object(t) = t {
                        if let Some(t) = t.get("base_url") {
                            if let json::Value::String(t) = t {
                                if !choose_quality_manually {
                                    best_audio = t.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    let video_url: Vec<_> = post_res.data.dash.video.iter().filter(|x| x.id == quality_id).collect();
    Ok((video_url[0].base_url.to_string(), best_audio))
}