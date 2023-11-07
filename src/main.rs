// 引入依赖库
use inquire::{
    validator::Validation,
    Text,
};
use fancy_regex::Regex;
use reqwest::blocking as req;
use reqwest::redirect::Policy;

// 常量部分，主要用于正则表达式匹配和B站API
const REG_BVID: &str = r"BV\w{10}";
const REG_AVID: &str = r"av\d{1,9}";
// const REG_URL: &str = r"(.*)bilibili.com/video/(BV\w{10}|av\d{1,9})";
const REG_URL: &str = r"(.*)bilibili.com/video/(BV\w{10}|av\d{1,9})(?=/|\?|$)";
const REG_SHORT_URL: &str = r"(http(s|)://|^)b23.tv/(\w+)";

const API_VIDEO_INFO: &str = "https://api.bilibili.com/x/web-interface/view";
const API_STREAM_URL: &str = "https://api.bilibili.com/x/player/wbi/playurl";
const API_USER_INFO: &str = "https://api.bilibili.com/x/web-interface/nav";

enum VideoId {
    Avid(u32),
    Bvid(String)
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
    let mut video_id = VideoId::Avid(0);

    let cookie = Text::new("请输入Cookie SESSDATA =").prompt().unwrap();

    // 询问+处理逻辑，当处理出错时（短链接404、长链接格式有误等正则查不出来等错误）循环提示用户重新输入
    while input_invalid {
        let res = video_inquirer.clone().prompt().unwrap();
        match parse_video_id(&res) {
            Ok(t) => {
                video_id = t;
                input_invalid = false;
            }
            Err(e) => println!("{}", e)
        }
    }
    match video_id {
        VideoId::Avid(t) => println!("{}",t),
        VideoId::Bvid(t) => println!("{}",t)
    }
}

// 将用户输入的视频url、短链接、av/bv号等统一处理成av/bv号，方便后续请求
fn parse_video_id(input: &str) -> Result<VideoId, &str> {
    let reg_bvid = Regex::new(REG_BVID).unwrap();
    let reg_avid = Regex::new(REG_AVID).unwrap();
    let reg_url = Regex::new(REG_URL).unwrap();
    let reg_short_url = Regex::new(REG_SHORT_URL).unwrap();
    let url_to_id = |a: &str| -> Result<VideoId, &str> {
        let processed_url = match reg_url.captures(a).unwrap() {
            Some(t) => t[0].to_string(),
            None => return Err("解析视频Url出错")
        };
        match reg_bvid.captures(&processed_url).unwrap() {
            Some(caps) => Ok(VideoId::Bvid(caps[0].to_string())),
            None => match reg_avid.captures(input).unwrap() {
                Some(caps) => Ok(VideoId::Avid(caps[0].to_string()[2..].parse::<u32>().unwrap())),
                None => Err("解析视频Url出错")
            }
        }
    };
    if reg_url.is_match(input).unwrap() {
        url_to_id(input)
    } else if reg_bvid.is_match(input).unwrap() {
        Ok(VideoId::Bvid(input.to_string()))}
    else if reg_avid.is_match(input).unwrap() {
        Ok(VideoId::Avid(input[2..].parse::<u32>().unwrap()))
    } else if reg_short_url.is_match(input).unwrap() {
        let processed_short_url = match reg_short_url.captures(input).unwrap() {
            Some(t) => t[0].to_string(),
            None => return Err("解析短Url出错")
        };
        match parse_short_url(&processed_short_url) {
            Some(t) => url_to_id(&t),
            None => Err("该b23.tv短链接无效")
        }
    } else {
        Err("视频链接无效")
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