//! Per-service language code tables and Chinese-variant helpers.
//!
//! Every table is transcribed from the corresponding `*TranslateType.swift` /
//! `+Language.swift` file in the macOS app so that request payloads match
//! byte-for-byte.

use crate::model::{Language, ServiceType};
use std::sync::OnceLock;

/// Looks up the service-specific code for a language.
pub fn default_language_code(service: ServiceType, language: Language) -> Option<String> {
    use ServiceType::*;
    let table: &[(Language, &str)] = match service {
        Youdao => YOUDAO,
        DeepL => DEEPL,
        Google => GOOGLE,
        Bing => BING,
        Baidu => BAIDU,
        Caiyun => CAIYUN,
        Tencent => TENCENT,
        Volcano => VOLCANO,
        Alibaba => ALIBABA,
        NiuTrans => NIUTRANS,
        Doubao => DOUBAO,
        // LLM services always accept the English language name.
        OpenAI | CustomOpenAI | BuiltInAI | Gemini | Claude | DeepSeek | Groq | Zhipu | MiniMax
        | GitHub | Ollama | ClaudeCode | CodexCLI | GitHubCopilot | Polishing | Summary => {
            return Some(language.english_name().to_string())
        }
        // MDict is a local dictionary: the lookup key is the query text, not
        // a language code, so any named language is accepted.
        MDict => return Some(language.english_name().to_string()),
        SystemSpeech => return Some(language.windows_locale().to_string()),
        // macOS-only: `DictionaryServices.framework` and the system translate
        // panel. They keep a slot in `ServiceType` so settings round-trip, but
        // no language maps to them here.
        AppleDictionary | Apple => &[],
    };
    table
        .iter()
        .find(|(l, _)| *l == language)
        .map(|(_, code)| (*code).to_string())
}

/// `EZLanguageManager.onlyContainsChineseLanguages(_:)`
pub fn only_chinese<const N: usize>(langs: [Language; N]) -> bool {
    langs
        .iter()
        .all(|l| matches!(l, Language::SimplifiedChinese | Language::TraditionalChinese))
}

/// Converts between Simplified and Traditional Chinese when the pair is
/// Chinese-only, mirroring `toSimplifiedChinese()` / `toTraditionalChinese()`.
///
/// When both sides are Chinese the text is converted to the target variant;
/// otherwise the text passes through unchanged.
pub fn convert_chinese_variant(text: &str, from: Language, to: Language) -> String {
    match (from, to) {
        (Language::SimplifiedChinese, Language::TraditionalChinese) => to_traditional(text),
        (Language::TraditionalChinese, Language::SimplifiedChinese) => to_simplified(text),
        _ => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Chinese variant conversion tables
// ---------------------------------------------------------------------------

/// Simplified -> Traditional pairs covering the characters that appear in
/// ordinary prose. Not exhaustive: rare variants fall through unchanged.
const S2T: &[(char, char)] = &[
    ('们', '們'), ('个', '個'), ('为', '為'), ('这', '這'), ('说', '說'), ('时', '時'),
    ('会', '會'), ('对', '對'), ('来', '來'), ('学', '學'), ('国', '國'), ('后', '後'),
    ('么', '麼'), ('过', '過'), ('样', '樣'), ('点', '點'), ('现', '現'), ('发', '發'),
    ('问', '問'), ('题', '題'), ('书', '書'), ('写', '寫'), ('听', '聽'), ('读', '讀'),
    ('讲', '講'), ('话', '話'), ('语', '語'), ('词', '詞'), ('译', '譯'), ('爱', '愛'),
    ('欢', '歡'), ('乐', '樂'), ('车', '車'), ('门', '門'), ('马', '馬'), ('鸟', '鳥'),
    ('鱼', '魚'), ('龙', '龍'), ('风', '風'), ('云', '雲'), ('电', '電'), ('头', '頭'),
    ('长', '長'), ('万', '萬'), ('与', '與'), ('专', '專'), ('业', '業'), ('东', '東'),
    ('丝', '絲'), ('严', '嚴'), ('丽', '麗'), ('义', '義'), ('乌', '烏'), ('乔', '喬'),
    ('习', '習'), ('乡', '鄉'), ('买', '買'), ('乱', '亂'), ('争', '爭'), ('于', '於'),
    ('亏', '虧'), ('亚', '亞'), ('产', '產'), ('亲', '親'), ('亿', '億'), ('仅', '僅'),
    ('从', '從'), ('仑', '侖'), ('仓', '倉'), ('仪', '儀'), ('价', '價'), ('众', '眾'),
    ('优', '優'), ('传', '傳'), ('伟', '偉'), ('体', '體'), ('余', '餘'), ('侠', '俠'),
    ('侣', '侶'), ('俭', '儉'), ('债', '債'), ('倾', '傾'), ('偿', '償'), ('储', '儲'),
    ('儿', '兒'), ('党', '黨'), ('兰', '蘭'), ('关', '關'), ('兴', '興'), ('养', '養'),
    ('兽', '獸'), ('内', '內'), ('冈', '岡'), ('册', '冊'), ('写', '寫'), ('军', '軍'),
    ('农', '農'), ('冯', '馮'), ('冲', '衝'), ('决', '決'), ('况', '況'), ('冻', '凍'),
    ('净', '淨'), ('凉', '涼'), ('减', '減'), ('凑', '湊'), ('凤', '鳳'), ('凭', '憑'),
    ('击', '擊'), ('则', '則'), ('刚', '剛'), ('创', '創'), ('删', '刪'), ('别', '別'),
    ('剂', '劑'), ('剐', '剮'), ('剑', '劍'), ('剧', '劇'), ('劝', '勸'), ('办', '辦'),
    ('务', '務'), ('动', '動'), ('励', '勵'), ('劲', '勁'), ('劳', '勞'), ('势', '勢'),
    ('勋', '勳'), ('匀', '勻'), ('区', '區'), ('医', '醫'), ('华', '華'), ('协', '協'),
    ('单', '單'), ('卖', '賣'), ('卢', '盧'), ('卫', '衛'), ('却', '卻'), ('厂', '廠'),
    ('厅', '廳'), ('历', '歷'), ('厉', '厲'), ('压', '壓'), ('厌', '厭'), ('县', '縣'),
    ('参', '參'), ('双', '雙'), ('发', '發'), ('变', '變'), ('叙', '敘'), ('叠', '疊'),
    ('号', '號'), ('叹', '嘆'), ('叽', '嘰'), ('吁', '籲'), ('吗', '嗎'), ('吨', '噸'),
    ('听', '聽'), ('启', '啟'), ('呜', '嗚'), ('员', '員'), ('呛', '嗆'), ('呜', '嗚'),
    ('咏', '詠'), ('响', '響'), ('哑', '啞'), ('唤', '喚'), ('啰', '囉'), ('喷', '噴'),
    ('嘱', '囑'), ('团', '團'), ('园', '園'), ('围', '圍'), ('图', '圖'), ('圆', '圓'),
    ('圣', '聖'), ('场', '場'), ('坏', '壞'), ('块', '塊'), ('坚', '堅'), ('坛', '壇'),
    ('坝', '壩'), ('坟', '墳'), ('坠', '墜'), ('垄', '壟'), ('垒', '壘'), ('垦', '墾'),
    ('垫', '墊'), ('埘', '塒'), ('墙', '牆'), ('壮', '壯'), ('声', '聲'), ('壳', '殼'),
    ('处', '處'), ('备', '備'), ('复', '復'), ('够', '夠'), ('头', '頭'), ('夹', '夾'),
    ('夺', '奪'), ('奋', '奮'), ('奖', '獎'), ('妇', '婦'), ('妈', '媽'), ('妆', '妝'),
    ('姗', '姍'), ('姜', '薑'), ('娄', '婁'), ('娅', '婭'), ('娇', '嬌'), ('娘', '娘'),
    ('娱', '娛'), ('婴', '嬰'), ('婵', '嬋'), ('孙', '孫'), ('宁', '寧'), ('宝', '寶'),
    ('实', '實'), ('宠', '寵'), ('审', '審'), ('宪', '憲'), ('宫', '宮'), ('宽', '寬'),
    ('宾', '賓'), ('对', '對'), ('寻', '尋'), ('导', '導'), ('寿', '壽'), ('将', '將'),
    ('尔', '爾'), ('尘', '塵'), ('尝', '嘗'), ('层', '層'), ('屉', '屜'), ('届', '屆'),
    ('属', '屬'), ('岁', '歲'), ('岂', '豈'), ('岗', '崗'), ('岛', '島'), ('岭', '嶺'),
    ('岳', '嶽'), ('峡', '峽'), ('峦', '巒'), ('币', '幣'), ('帅', '帥'), ('师', '師'),
    ('帐', '帳'), ('帘', '簾'), ('帜', '幟'), ('带', '帶'), ('帮', '幫'), ('干', '幹'),
    ('并', '並'), ('广', '廣'), ('庄', '莊'), ('庆', '慶'), ('库', '庫'), ('应', '應'),
    ('废', '廢'), ('开', '開'), ('异', '異'), ('弃', '棄'), ('张', '張'), ('弹', '彈'),
    ('强', '強'), ('归', '歸'), ('当', '當'), ('录', '錄'), ('彻', '徹'), ('径', '徑'),
    ('徕', '徠'), ('忆', '憶'), ('忏', '懺'), ('怀', '懷'), ('态', '態'), ('怂', '慫'),
    ('总', '總'), ('恋', '戀'), ('恳', '懇'), ('恶', '惡'), ('恼', '惱'), ('悦', '悅'),
    ('悬', '懸'), ('惊', '驚'), ('惧', '懼'), ('惨', '慘'), ('惩', '懲'), ('惯', '慣'),
    ('愈', '癒'), ('愿', '願'), ('懒', '懶'), ('戏', '戲'), ('战', '戰'), ('户', '戶'),
    ('扑', '撲'), ('执', '執'), ('扩', '擴'), ('扫', '掃'), ('扬', '揚'), ('扰', '擾'),
    ('抚', '撫'), ('抛', '拋'), ('抢', '搶'), ('护', '護'), ('报', '報'), ('担', '擔'),
    ('拟', '擬'), ('拥', '擁'), ('拨', '撥'), ('择', '擇'), ('挂', '掛'), ('挚', '摯'),
    ('损', '損'), ('换', '換'), ('据', '據'), ('掳', '擄'), ('掷', '擲'), ('揽', '攬'),
    ('撑', '撐'), ('撵', '攆'), ('擞', '擻'), ('摆', '擺'), ('摇', '搖'), ('携', '攜'),
    ('摄', '攝'), ('摊', '攤'), ('撑', '撐'), ('撒', '撒'), ('攒', '攢'), ('敌', '敵'),
    ('数', '數'), ('敛', '斂'), ('断', '斷'), ('无', '無'), ('旧', '舊'), ('时', '時'),
    ('旷', '曠'), ('昙', '曇'), ('显', '顯'), ('晓', '曉'), ('晕', '暈'), ('暂', '暫'),
    ('术', '術'), ('机', '機'), ('杀', '殺'), ('杂', '雜'), ('权', '權'), ('条', '條'),
    ('来', '來'), ('杨', '楊'), ('杰', '傑'), ('极', '極'), ('构', '構'), ('枪', '槍'),
    ('枫', '楓'), ('柜', '櫃'), ('标', '標'), ('栈', '棧'), ('栏', '欄'), ('树', '樹'),
    ('样', '樣'), ('档', '檔'), ('桥', '橋'), ('梦', '夢'), ('检', '檢'), ('椭', '橢'),
    ('楼', '樓'), ('概', '概'), ('槛', '檻'), ('横', '橫'), ('欢', '歡'), ('欧', '歐'),
    ('残', '殘'), ('殴', '毆'), ('毁', '毀'), ('毕', '畢'), ('毙', '斃'), ('气', '氣'),
    ('汇', '匯'), ('汉', '漢'), ('汤', '湯'), ('汹', '洶'), ('沟', '溝'), ('没', '沒'),
    ('沦', '淪'), ('泪', '淚'), ('泼', '潑'), ('泽', '澤'), ('洁', '潔'), ('洒', '灑'),
    ('浅', '淺'), ('浆', '漿'), ('浊', '濁'), ('测', '測'), ('济', '濟'), ('浏', '瀏'),
    ('浓', '濃'), ('涂', '塗'), ('涛', '濤'), ('涝', '澇'), ('润', '潤'), ('涨', '漲'),
    ('渐', '漸'), ('湿', '濕'), ('温', '溫'), ('湾', '灣'), ('溃', '潰'), ('滚', '滾'),
    ('满', '滿'), ('滤', '濾'), ('滩', '灘'), ('滨', '濱'), ('潜', '潛'), ('澜', '瀾'),
    ('灭', '滅'), ('灯', '燈'), ('灵', '靈'), ('灾', '災'), ('炉', '爐'), ('炼', '煉'),
    ('烂', '爛'), ('烛', '燭'), ('烟', '煙'), ('烦', '煩'), ('烧', '燒'), ('热', '熱'),
    ('焕', '煥'), ('爱', '愛'), ('爷', '爺'), ('牍', '牘'), ('牵', '牽'), ('犊', '犢'),
    ('状', '狀'), ('犹', '猶'), ('狈', '狽'), ('狞', '獰'), ('独', '獨'), ('狭', '狹'),
    ('狮', '獅'), ('猎', '獵'), ('猪', '豬'), ('献', '獻'), ('玛', '瑪'), ('环', '環'),
    ('现', '現'), ('玺', '璽'), ('琳', '琳'), ('瓒', '瓚'), ('瓮', '甕'), ('产', '產'),
    ('画', '畫'), ('畅', '暢'), ('畴', '疇'), ('疗', '療'), ('疟', '瘧'), ('疮', '瘡'),
    ('疯', '瘋'), ('痉', '痙'), ('痒', '癢'), ('痪', '瘓'), ('痴', '痴'), ('瘫', '癱'),
    ('瘾', '癮'), ('皑', '皚'), ('皱', '皺'), ('盗', '盜'), ('盖', '蓋'), ('盘', '盤'),
    ('监', '監'), ('盖', '蓋'), ('眍', '瞘'), ('睁', '睜'), ('瞒', '瞞'), ('矫', '矯'),
    ('硕', '碩'), ('碍', '礙'), ('礼', '禮'), ('祷', '禱'), ('禄', '祿'), ('祸', '禍'),
    ('离', '離'), ('种', '種'), ('积', '積'), ('称', '稱'), ('秽', '穢'), ('稳', '穩'),
    ('穷', '窮'), ('窃', '竊'), ('窜', '竄'), ('窝', '窩'), ('窥', '窺'), ('竖', '豎'),
    ('竞', '競'), ('笋', '筍'), ('笔', '筆'), ('笼', '籠'), ('筛', '篩'), ('筹', '籌'),
    ('签', '簽'), ('简', '簡'), ('篮', '籃'), ('籁', '籟'), ('类', '類'), ('粮', '糧'),
    ('纠', '糾'), ('纪', '紀'), ('纬', '緯'), ('纯', '純'), ('纱', '紗'), ('纲', '綱'),
    ('纳', '納'), ('纵', '縱'), ('纷', '紛'), ('纸', '紙'), ('纹', '紋'), ('纺', '紡'),
    ('纽', '紐'), ('线', '線'), ('练', '練'), ('组', '組'), ('绅', '紳'), ('细', '細'),
    ('织', '織'), ('终', '終'), ('绊', '絆'), ('绍', '紹'), ('经', '經'), ('绑', '綁'),
    ('绒', '絨'), ('结', '結'), ('绕', '繞'), ('绘', '繪'), ('给', '給'), ('络', '絡'),
    ('绝', '絕'), ('绞', '絞'), ('统', '統'), ('绣', '繡'), ('继', '繼'), ('绩', '績'),
    ('绪', '緒'), ('续', '續'), ('绳', '繩'), ('维', '維'), ('绵', '綿'), ('综', '綜'),
    ('绿', '綠'), ('缓', '緩'), ('编', '編'), ('缘', '緣'), ('缚', '縛'), ('缝', '縫'),
    ('缩', '縮'), ('缴', '繳'), ('网', '網'), ('罗', '羅'), ('罚', '罰'), ('罢', '罷'),
    ('羡', '羨'), ('翘', '翹'), ('耸', '聳'), ('耻', '恥'), ('聂', '聶'), ('聋', '聾'),
    ('职', '職'), ('联', '聯'), ('聪', '聰'), ('肃', '肅'), ('肠', '腸'), ('肤', '膚'),
    ('肿', '腫'), ('胀', '脹'), ('胁', '脅'), ('胆', '膽'), ('胜', '勝'), ('胶', '膠'),
    ('脉', '脈'), ('脏', '臟'), ('脑', '腦'), ('脓', '膿'), ('脸', '臉'), ('腊', '臘'),
    ('腻', '膩'), ('腾', '騰'), ('舰', '艦'), ('舱', '艙'), ('艰', '艱'), ('艳', '艷'),
    ('艺', '藝'), ('节', '節'), ('芜', '蕪'), ('芦', '蘆'), ('苍', '蒼'), ('苏', '蘇'),
    ('苹', '蘋'), ('范', '範'), ('茧', '繭'), ('荆', '荊'), ('荐', '薦'), ('荚', '莢'),
    ('荟', '薈'), ('荞', '蕎'), ('荡', '蕩'), ('荣', '榮'), ('药', '藥'), ('莱', '萊'),
    ('莲', '蓮'), ('获', '獲'), ('莹', '瑩'), ('萝', '蘿'), ('营', '營'), ('萧', '蕭'),
    ('萨', '薩'), ('葱', '蔥'), ('蒋', '蔣'), ('蓝', '藍'), ('蓟', '薊'), ('蔷', '薔'),
    ('蔼', '藹'), ('蕴', '蘊'), ('薮', '藪'), ('虚', '虛'), ('虫', '蟲'), ('虽', '雖'),
    ('蚀', '蝕'), ('蚁', '蟻'), ('蚕', '蠶'), ('蛊', '蠱'), ('蛮', '蠻'), ('蛊', '蠱'),
    ('蝇', '蠅'), ('蝉', '蟬'), ('蜡', '蠟'), ('蝇', '蠅'), ('蝉', '蟬'), ('衅', '釁'),
    ('衔', '銜'), ('补', '補'), ('衬', '襯'), ('袄', '襖'), ('装', '裝'), ('裤', '褲'),
    ('见', '見'), ('观', '觀'), ('规', '規'), ('觅', '覓'), ('视', '視'), ('览', '覽'),
    ('觉', '覺'), ('觊', '覬'), ('觐', '覲'), ('触', '觸'), ('觞', '觴'), ('订', '訂'),
    ('计', '計'), ('认', '認'), ('讥', '譏'), ('讨', '討'), ('让', '讓'), ('训', '訓'),
    ('议', '議'), ('讯', '訊'), ('记', '記'), ('讲', '講'), ('讳', '諱'), ('讶', '訝'),
    ('许', '許'), ('论', '論'), ('讼', '訟'), ('讽', '諷'), ('设', '設'), ('访', '訪'),
    ('诀', '訣'), ('证', '證'), ('评', '評'), ('识', '識'), ('诈', '詐'), ('诉', '訴'),
    ('诊', '診'), ('诋', '詆'), ('词', '詞'), ('译', '譯'), ('试', '試'), ('诗', '詩'),
    ('诘', '詰'), ('诚', '誠'), ('话', '話'), ('诞', '誕'), ('询', '詢'), ('该', '該'),
    ('详', '詳'), ('诫', '誡'), ('诬', '誣'), ('语', '語'), ('误', '誤'), ('诱', '誘'),
    ('说', '說'), ('请', '請'), ('诸', '諸'), ('诺', '諾'), ('读', '讀'), ('课', '課'),
    ('谁', '誰'), ('调', '調'), ('谅', '諒'), ('谊', '誼'), ('谋', '謀'), ('谎', '謊'),
    ('谐', '諧'), ('谓', '謂'), ('谚', '諺'), ('谜', '謎'), ('谢', '謝'), ('谣', '謠'),
    ('谨', '謹'), ('谱', '譜'), ('谴', '譴'), ('谷', '穀'), ('豮', '豶'), ('贝', '貝'),
    ('贞', '貞'), ('负', '負'), ('贡', '貢'), ('财', '財'), ('责', '責'), ('贤', '賢'),
    ('败', '敗'), ('账', '賬'), ('货', '貨'), ('质', '質'), ('贩', '販'), ('贪', '貪'),
    ('贫', '貧'), ('贬', '貶'), ('购', '購'), ('贯', '貫'), ('贮', '貯'), ('贰', '貳'),
    ('贱', '賤'), ('贴', '貼'), ('贵', '貴'), ('贷', '貸'), ('贸', '貿'), ('费', '費'),
    ('贺', '賀'), ('贼', '賊'), ('贾', '賈'), ('贿', '賄'), ('赁', '賃'), ('赂', '賂'),
    ('资', '資'), ('赋', '賦'), ('赌', '賭'), ('赏', '賞'), ('赐', '賜'), ('赔', '賠'),
    ('赖', '賴'), ('赚', '賺'), ('赛', '賽'), ('赞', '贊'), ('赠', '贈'), ('赡', '贍'),
    ('赢', '贏'), ('赵', '趙'), ('赶', '趕'), ('趋', '趨'), ('趟', '趟'), ('跃', '躍'),
    ('践', '踐'), ('踪', '蹤'), ('踹', '踹'), ('蹿', '躥'), ('躏', '躪'), ('躯', '軀'),
    ('车', '車'), ('轧', '軋'), ('轨', '軌'), ('轩', '軒'), ('转', '轉'), ('轮', '輪'),
    ('软', '軟'), ('轰', '轟'), ('轴', '軸'), ('轻', '輕'), ('载', '載'), ('较', '較'),
    ('辅', '輔'), ('辆', '輛'), ('辈', '輩'), ('辉', '輝'), ('辐', '輻'), ('输', '輸'),
    ('辖', '轄'), ('辗', '輾'), ('辘', '轆'), ('辙', '轍'), ('辞', '辭'), ('辩', '辯'),
    ('边', '邊'), ('辽', '遼'), ('达', '達'), ('迁', '遷'), ('过', '過'), ('迈', '邁'),
    ('运', '運'), ('还', '還'), ('这', '這'), ('进', '進'), ('远', '遠'), ('违', '違'),
    ('连', '連'), ('迟', '遲'), ('迹', '跡'), ('适', '適'), ('选', '選'), ('逊', '遜'),
    ('递', '遞'), ('逻', '邏'), ('遗', '遺'), ('遥', '遙'), ('邓', '鄧'), ('郑', '鄭'),
    ('邻', '鄰'), ('郁', '鬱'), ('酝', '醞'), ('酱', '醬'), ('酿', '釀'), ('释', '釋'),
    ('里', '裡'), ('鉴', '鑒'), ('针', '針'), ('钉', '釘'), ('钓', '釣'), ('钙', '鈣'),
    ('钝', '鈍'), ('钞', '鈔'), ('钟', '鐘'), ('钠', '鈉'), ('钢', '鋼'), ('钥', '鑰'),
    ('钦', '欽'), ('钧', '鈞'), ('钩', '鉤'), ('钱', '錢'), ('钳', '鉗'), ('钻', '鑽'),
    ('铁', '鐵'), ('铃', '鈴'), ('铅', '鉛'), ('铜', '銅'), ('铝', '鋁'), ('铠', '鎧'),
    ('银', '銀'), ('铸', '鑄'), ('铺', '鋪'), ('链', '鏈'), ('销', '銷'), ('锁', '鎖'),
    ('锅', '鍋'), ('锋', '鋒'), ('锐', '銳'), ('错', '錯'), ('锡', '錫'), ('锣', '鑼'),
    ('锤', '錘'), ('锦', '錦'), ('键', '鍵'), ('锯', '鋸'), ('镀', '鍍'), ('镁', '鎂'),
    ('镇', '鎮'), ('镊', '鑷'), ('镜', '鏡'), ('镣', '鐐'), ('长', '長'), ('门', '門'),
    ('闪', '閃'), ('闭', '閉'), ('问', '問'), ('闯', '闖'), ('闲', '閒'), ('间', '間'),
    ('闷', '悶'), ('闸', '閘'), ('闹', '鬧'), ('闻', '聞'), ('阀', '閥'), ('阁', '閣'),
    ('阅', '閱'), ('阎', '閻'), ('阐', '闡'), ('阑', '闌'), ('阔', '闊'), ('阙', '闕'),
    ('队', '隊'), ('阳', '陽'), ('阴', '陰'), ('阵', '陣'), ('阶', '階'), ('际', '際'),
    ('陆', '陸'), ('陈', '陳'), ('陕', '陝'), ('隐', '隱'), ('难', '難'), ('雏', '雛'),
    ('雾', '霧'), ('霁', '霽'), ('霉', '黴'), ('静', '靜'), ('韩', '韓'), ('韧', '韌'),
    ('顶', '頂'), ('顷', '頃'), ('项', '項'), ('顺', '順'), ('须', '須'), ('顽', '頑'),
    ('顾', '顧'), ('顿', '頓'), ('颁', '頒'), ('颂', '頌'), ('预', '預'), ('颅', '顱'),
    ('领', '領'), ('颇', '頗'), ('颈', '頸'), ('颊', '頰'), ('频', '頻'), ('颗', '顆'),
    ('题', '題'), ('颜', '顏'), ('额', '額'), ('颠', '顛'), ('颤', '顫'), ('风', '風'),
    ('飞', '飛'), ('飨', '饗'), ('饭', '飯'), ('饮', '飲'), ('饰', '飾'), ('饱', '飽'),
    ('饲', '飼'), ('饶', '饒'), ('饺', '餃'), ('饼', '餅'), ('饿', '餓'), ('馅', '餡'),
    ('馆', '館'), ('馈', '饋'), ('馋', '饞'), ('馊', '餿'), ('马', '馬'), ('驭', '馭'),
    ('驯', '馴'), ('驰', '馳'), ('驱', '驅'), ('驳', '駁'), ('驴', '驢'), ('驶', '駛'),
    ('驹', '駒'), ('驻', '駐'), ('驼', '駝'), ('驾', '駕'), ('骂', '罵'), ('骄', '驕'),
    ('骆', '駱'), ('骇', '駭'), ('骏', '駿'), ('骑', '騎'), ('骗', '騙'), ('骚', '騷'),
    ('骤', '驟'), ('骨', '骨'), ('髅', '髏'), ('髌', '髕'), ('鬓', '鬢'), ('鱼', '魚'),
    ('鲁', '魯'), ('鲜', '鮮'), ('鲤', '鯉'), ('鲨', '鯊'), ('鲸', '鯨'), ('鳄', '鱷'),
    ('鸟', '鳥'), ('鸠', '鳩'), ('鸡', '雞'), ('鸣', '鳴'), ('鸥', '鷗'), ('鸦', '鴉'),
    ('鸽', '鴿'), ('鹅', '鵝'), ('鹉', '鵡'), ('鹊', '鵲'), ('鹤', '鶴'), ('鹰', '鷹'),
    ('鹿', '鹿'), ('麦', '麥'), ('黄', '黃'), ('黉', '黌'), ('齐', '齊'), ('齿', '齒'),
    ('龄', '齡'), ('龙', '龍'), ('龟', '龜'),
];

/// Traditional -> Simplified, derived from `S2T` plus one-to-many cases.
fn t2s_table() -> &'static std::collections::HashMap<char, char> {
    static TABLE: OnceLock<std::collections::HashMap<char, char>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut m = std::collections::HashMap::with_capacity(S2T.len());
        // Insert in order, so the *last* mapping for a shared traditional
        // char wins; that is the more common simplification.
        for (s, t) in S2T {
            m.insert(*t, *s);
        }
        // Explicit overrides for characters whose simplification is not the
        // reverse of the pair above.
        for (t, s) in [
            ('幹', '干'), ('乾', '干'), ('發', '发'), ('臺', '台'), ('檯', '台'),
            ('颱', '台'), ('裡', '里'), ('麵', '面'), ('麪', '面'), ('後', '后'),
            ('隻', '只'), ('祗', '只'), ('隻', '只'), ('週', '周'), ('週', '周'),
            ('鐘', '钟'), ('鍾', '钟'), ('徵', '征'), ('癥', '症'), ('錶', '表'),
            ('佈', '布'), ('佔', '占'), ('決', '决'), ('況', '况'), ('鬥', '斗'),
            ('網', '网'), ('綢', '绸'), ('纜', '缆'), ('襪', '袜'), ('褲', '裤'),
        ] {
            m.insert(t, s);
        }
        m
    })
}

fn s2t_table() -> &'static std::collections::HashMap<char, char> {
    static TABLE: OnceLock<std::collections::HashMap<char, char>> = OnceLock::new();
    TABLE.get_or_init(|| S2T.iter().copied().collect())
}

/// Simplified -> Traditional.
pub fn to_traditional(text: &str) -> String {
    let table = s2t_table();
    text.chars()
        .map(|c| table.get(&c).copied().unwrap_or(c))
        .collect()
}

/// Traditional -> Simplified.
pub fn to_simplified(text: &str) -> String {
    let table = t2s_table();
    text.chars()
        .map(|c| table.get(&c).copied().unwrap_or(c))
        .collect()
}

// ---------------------------------------------------------------------------
// Language tables
// ---------------------------------------------------------------------------

/// Youdao web translation API (`YoudaoService.languagesDictionary`).
const YOUDAO: &[(Language, &str)] = &[
    (Language::SimplifiedChinese, "zh-CHS"),
    (Language::TraditionalChinese, "zh-CHT"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Portuguese, "pt"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Arabic, "ar"),
    (Language::Thai, "th"),
    (Language::Dutch, "nl"),
    (Language::Indonesian, "id"),
    (Language::Vietnamese, "vi"),
];

const DEEPL: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh-hans"),
    (Language::TraditionalChinese, "zh-hant"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Catalan, "ca"),
    (Language::Portuguese, "pt-PT"),
    (Language::BrazilianPortuguese, "pt-BR"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Swedish, "sv"),
    (Language::Romanian, "ro"),
    (Language::Slovak, "sk"),
    (Language::Dutch, "nl"),
    (Language::Hungarian, "hu"),
    (Language::Greek, "el"),
    (Language::Danish, "da"),
    (Language::Finnish, "fi"),
    (Language::Polish, "pl"),
    (Language::Czech, "cs"),
    (Language::Turkish, "tr"),
    (Language::Lithuanian, "lt"),
    (Language::Latvian, "lv"),
    (Language::Ukrainian, "uk"),
    (Language::Bulgarian, "bg"),
    (Language::Indonesian, "id"),
    (Language::Slovenian, "sl"),
    (Language::Estonian, "et"),
    (Language::Norwegian, "nb"),
    (Language::Arabic, "ar"),
    (Language::Vietnamese, "vi"),
    (Language::Hebrew, "he"),
    (Language::Thai, "th"),
];

const GOOGLE: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh-CN"),
    (Language::TraditionalChinese, "zh-TW"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Catalan, "ca"),
    (Language::Portuguese, "pt-PT"),
    (Language::BrazilianPortuguese, "pt"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Arabic, "ar"),
    (Language::Swedish, "sv"),
    (Language::Romanian, "ro"),
    (Language::Thai, "th"),
    (Language::Slovak, "sk"),
    (Language::Dutch, "nl"),
    (Language::Hungarian, "hu"),
    (Language::Greek, "el"),
    (Language::Danish, "da"),
    (Language::Finnish, "fi"),
    (Language::Polish, "pl"),
    (Language::Czech, "cs"),
    (Language::Turkish, "tr"),
    (Language::Lithuanian, "lt"),
    (Language::Latvian, "lv"),
    (Language::Ukrainian, "uk"),
    (Language::Bulgarian, "bg"),
    (Language::Indonesian, "id"),
    (Language::Malay, "ms"),
    (Language::Slovenian, "sl"),
    (Language::Estonian, "et"),
    (Language::Vietnamese, "vi"),
    (Language::Persian, "fa"),
    (Language::Hindi, "hi"),
    (Language::Telugu, "te"),
    (Language::Tamil, "ta"),
    (Language::Urdu, "ur"),
    (Language::Filipino, "tl"),
    (Language::Khmer, "km"),
    (Language::Lao, "lo"),
    (Language::Bengali, "bn"),
    (Language::Burmese, "my"),
    (Language::Norwegian, "no"),
    (Language::Serbian, "sr"),
    (Language::Croatian, "hr"),
    (Language::Mongolian, "mn"),
    (Language::Hebrew, "iw"),
    (Language::Georgian, "ka"),
    (Language::Uyghur, "ug"),
];

const BING: &[(Language, &str)] = &[
    (Language::Auto, "auto-detect"),
    (Language::SimplifiedChinese, "zh-Hans"),
    (Language::TraditionalChinese, "zh-Hant"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Catalan, "ca"),
    (Language::Portuguese, "pt-PT"),
    (Language::BrazilianPortuguese, "pt"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Arabic, "ar"),
    (Language::Swedish, "sv"),
    (Language::Romanian, "ro"),
    (Language::Thai, "th"),
    (Language::Slovak, "sk"),
    (Language::Dutch, "nl"),
    (Language::Hungarian, "hu"),
    (Language::Greek, "el"),
    (Language::Danish, "da"),
    (Language::Finnish, "fi"),
    (Language::Polish, "pl"),
    (Language::Czech, "cs"),
    (Language::Turkish, "tr"),
    (Language::Lithuanian, "lt"),
    (Language::Latvian, "lv"),
    (Language::Ukrainian, "uk"),
    (Language::Bulgarian, "bg"),
    (Language::Indonesian, "id"),
    (Language::Malay, "ms"),
    (Language::Slovenian, "sl"),
    (Language::Estonian, "et"),
    (Language::Vietnamese, "vi"),
    (Language::Persian, "fa"),
    (Language::Hindi, "hi"),
    (Language::Telugu, "te"),
    (Language::Tamil, "ta"),
    (Language::Urdu, "ur"),
    (Language::Filipino, "fil"),
    (Language::Khmer, "km"),
    (Language::Lao, "lo"),
    (Language::Bengali, "bn"),
    (Language::Burmese, "my"),
    (Language::Norwegian, "nb"),
    (Language::Serbian, "sr-Cyrl"),
    (Language::Croatian, "hr"),
    (Language::Mongolian, "mn-Mong"),
    (Language::Hebrew, "he"),
    (Language::Georgian, "ka"),
    (Language::Uyghur, "ug"),
];

const BAIDU: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh"),
    (Language::ClassicalChinese, "wyw"),
    (Language::TraditionalChinese, "cht"),
    (Language::English, "en"),
    (Language::Japanese, "jp"),
    (Language::Korean, "kor"),
    (Language::French, "fra"),
    (Language::Spanish, "spa"),
    (Language::Portuguese, "pt"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Arabic, "ara"),
    (Language::Swedish, "swe"),
    (Language::Romanian, "rom"),
    (Language::Thai, "th"),
    (Language::Slovak, "slo"),
    (Language::Dutch, "nl"),
    (Language::Hungarian, "hu"),
    (Language::Greek, "el"),
    (Language::Danish, "dan"),
    (Language::Finnish, "fin"),
    (Language::Polish, "pl"),
    (Language::Czech, "cs"),
    (Language::Bulgarian, "bul"),
    (Language::Estonian, "est"),
    (Language::Vietnamese, "vie"),
    // Uncommon tier — ordinary accounts may receive error 58001.
    (Language::Catalan, "cat"),
    (Language::BrazilianPortuguese, "pot"),
    (Language::Turkish, "tr"),
    (Language::Lithuanian, "lit"),
    (Language::Latvian, "lav"),
    (Language::Ukrainian, "ukr"),
    (Language::Indonesian, "id"),
    (Language::Malay, "msa"),
    (Language::Slovenian, "slv"),
    (Language::Persian, "per"),
    (Language::Hindi, "hin"),
    (Language::Telugu, "tel"),
    (Language::Tamil, "tam"),
    (Language::Urdu, "urd"),
    (Language::Filipino, "fil"),
    (Language::Khmer, "khm"),
    (Language::Lao, "lo"),
    (Language::Bengali, "ben"),
    (Language::Burmese, "bur"),
    (Language::Norwegian, "nor"),
    (Language::Serbian, "srp"),
    (Language::Croatian, "hrv"),
    (Language::Mongolian, "mon"),
    (Language::Hebrew, "heb"),
    (Language::Georgian, "geo"),
];

const CAIYUN: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh"),
    (Language::TraditionalChinese, "zh-Hant"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::German, "de"),
    (Language::Spanish, "es"),
    (Language::French, "fr"),
    (Language::Italian, "it"),
    (Language::Portuguese, "pt"),
    (Language::Russian, "ru"),
    (Language::Turkish, "tr"),
    (Language::Vietnamese, "vi"),
];

const TENCENT: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh"),
    (Language::TraditionalChinese, "zh-TW"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Turkish, "tr"),
    (Language::Russian, "ru"),
    (Language::Portuguese, "pt"),
    (Language::Vietnamese, "vi"),
    (Language::Indonesian, "id"),
    (Language::Thai, "th"),
    (Language::Malay, "ms"),
    (Language::Arabic, "ar"),
    (Language::Hindi, "hi"),
];

const VOLCANO: &[(Language, &str)] = &[
    (Language::SimplifiedChinese, "zh"),
    (Language::TraditionalChinese, "zh-Hant"),
    (Language::Vietnamese, "vi"),
    (Language::Italian, "it"),
    (Language::Indonesian, "id"),
    (Language::Hindi, "hi"),
    (Language::English, "en"),
    (Language::Hebrew, "he"),
    (Language::Spanish, "es"),
    (Language::Ukrainian, "uk"),
    (Language::Urdu, "ur"),
    (Language::Turkish, "tr"),
    (Language::Thai, "th"),
    (Language::Tamil, "ta"),
    (Language::Telugu, "te"),
    (Language::Slovenian, "sl"),
    (Language::Slovak, "sk"),
    (Language::Swedish, "sv"),
    (Language::Japanese, "ja"),
    (Language::Portuguese, "pt"),
    (Language::Norwegian, "no"),
    (Language::Burmese, "my"),
    (Language::Bengali, "bn"),
    (Language::Mongolian, "mn"),
    (Language::Malay, "ms"),
    (Language::Romanian, "ro"),
    (Language::Lithuanian, "lt"),
    (Language::Latvian, "lv"),
    (Language::Lao, "lo"),
    (Language::Croatian, "hr"),
    (Language::Czech, "cs"),
    (Language::Dutch, "nl"),
    (Language::Korean, "ko"),
    (Language::Khmer, "km"),
    (Language::Finnish, "fi"),
    (Language::French, "fr"),
    (Language::Russian, "ru"),
    (Language::German, "de"),
    (Language::Danish, "da"),
    (Language::Persian, "fa"),
    (Language::Polish, "pl"),
    (Language::Bulgarian, "bg"),
    (Language::Arabic, "ar"),
    (Language::Estonian, "et"),
    (Language::ClassicalChinese, "lzh"),
    (Language::Serbian, "sr"),
    (Language::Hungarian, "hu"),
    (Language::Georgian, "ka"),
    (Language::Uyghur, "ug"),
];

const ALIBABA: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh"),
    (Language::TraditionalChinese, "zh"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Catalan, "ca"),
    (Language::Portuguese, "pt"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Arabic, "ar"),
    (Language::Swedish, "sv"),
    (Language::Romanian, "ro"),
    (Language::Thai, "th"),
    (Language::Slovak, "sk"),
    (Language::Dutch, "nl"),
    (Language::Hungarian, "hu"),
    (Language::Greek, "el"),
    (Language::Danish, "da"),
    (Language::Finnish, "fi"),
    (Language::Polish, "pl"),
    (Language::Czech, "cs"),
    (Language::Turkish, "tr"),
    (Language::Lithuanian, "lt"),
    (Language::Latvian, "lv"),
    (Language::Bulgarian, "bg"),
    (Language::Malay, "ms"),
    (Language::Slovenian, "sl"),
    (Language::Estonian, "et"),
    (Language::Vietnamese, "vi"),
    (Language::Persian, "fa"),
    (Language::Hindi, "hi"),
    (Language::Telugu, "te"),
    (Language::Tamil, "ta"),
    (Language::Urdu, "ur"),
    (Language::Filipino, "fil"),
    (Language::Khmer, "km"),
    (Language::Lao, "lo"),
    (Language::Bengali, "bn"),
    (Language::Burmese, "my"),
    (Language::Norwegian, "no"),
    (Language::Croatian, "hbs"),
    (Language::Mongolian, "mn"),
    (Language::Hebrew, "he"),
    (Language::Georgian, "ka"),
];

const NIUTRANS: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh"),
    (Language::TraditionalChinese, "cht"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Catalan, "ca"),
    (Language::Portuguese, "pt"),
    (Language::BrazilianPortuguese, "pt-BR"),
    (Language::Italian, "it"),
    (Language::German, "de"),
    (Language::Russian, "ru"),
    (Language::Arabic, "ar"),
    (Language::Swedish, "sv"),
    (Language::Romanian, "ro"),
    (Language::Thai, "th"),
    (Language::Slovak, "sk"),
    (Language::Dutch, "nl"),
    (Language::Hungarian, "hu"),
    (Language::Greek, "el"),
    (Language::Danish, "da"),
    (Language::Finnish, "fi"),
    (Language::Polish, "pl"),
    (Language::Czech, "cs"),
    (Language::Turkish, "tr"),
    (Language::Lithuanian, "lt"),
    (Language::Latvian, "lv"),
    (Language::Ukrainian, "uk"),
    (Language::Bulgarian, "bg"),
    (Language::Indonesian, "id"),
    (Language::Malay, "ms"),
    (Language::Slovenian, "sl"),
    (Language::Estonian, "et"),
    (Language::Vietnamese, "vi"),
    (Language::Persian, "fa"),
    (Language::Hindi, "hi"),
    (Language::Telugu, "te"),
    (Language::Tamil, "ta"),
    (Language::Urdu, "ur"),
    (Language::Filipino, "fil"),
    (Language::Khmer, "km"),
    (Language::Lao, "lo"),
    (Language::Bengali, "bn"),
    (Language::Burmese, "my"),
    (Language::Norwegian, "no"),
    (Language::Serbian, "sr"),
    (Language::Croatian, "hr"),
    (Language::Mongolian, "mn"),
    (Language::Hebrew, "he"),
    (Language::Georgian, "jy"),
];

const DOUBAO: &[(Language, &str)] = &[
    (Language::Auto, "auto"),
    (Language::SimplifiedChinese, "zh"),
    (Language::TraditionalChinese, "zh-Hant"),
    (Language::English, "en"),
    (Language::Japanese, "ja"),
    (Language::Korean, "ko"),
    (Language::German, "de"),
    (Language::French, "fr"),
    (Language::Spanish, "es"),
    (Language::Italian, "it"),
    (Language::Portuguese, "pt"),
    (Language::Russian, "ru"),
    (Language::Thai, "th"),
    (Language::Vietnamese, "vi"),
    (Language::Arabic, "ar"),
    (Language::Czech, "cs"),
    (Language::Danish, "da"),
    (Language::Finnish, "fi"),
    (Language::Croatian, "hr"),
    (Language::Hungarian, "hu"),
    (Language::Indonesian, "id"),
    (Language::Malay, "ms"),
    (Language::Norwegian, "nb"),
    (Language::Dutch, "nl"),
    (Language::Polish, "pl"),
    (Language::Romanian, "ro"),
    (Language::Swedish, "sv"),
    (Language::Turkish, "tr"),
    (Language::Ukrainian, "uk"),
];

/// Tencent's supported source -> target matrix, from
/// `TencentTranslateType.supportedTypes`.
///
/// Upstream's `transType` accepts a pair when the matrix lists it, *or* the two
/// languages are equal, *or* both are Chinese — Tencent translates zh <-> zh-TW
/// even though the documentation does not say so.
pub fn tencent_supports_pair(from: Language, to: Language) -> bool {
    use Language::*;
    // `supportedTypes[.auto]` is nil upstream, so an auto source never matches.
    if from == Auto {
        return false;
    }
    if from == to || only_chinese([from, to]) {
        return true;
    }
    let targets: &[Language] = match from {
        SimplifiedChinese | TraditionalChinese => &[
            English, Japanese, Korean, French, Spanish, Italian, German, Turkish, Russian,
            Portuguese, Vietnamese, Indonesian, Thai, Malay,
        ],
        English => &[
            SimplifiedChinese, TraditionalChinese, Japanese, Korean, French, Spanish, Italian,
            German, Turkish, Russian, Portuguese, Vietnamese, Indonesian, Thai, Malay, Arabic,
            Hindi,
        ],
        Japanese => &[SimplifiedChinese, TraditionalChinese, English, Korean],
        Korean => &[SimplifiedChinese, TraditionalChinese, English, Japanese],
        French | Spanish | Italian | German | Turkish | Russian => &[
            SimplifiedChinese, TraditionalChinese, English, French, Spanish, Italian, German,
            Turkish, Russian, Portuguese,
        ],
        Portuguese => &[
            SimplifiedChinese, TraditionalChinese, English, French, Spanish, Italian, German,
            Turkish, Russian,
        ],
        Vietnamese | Indonesian | Thai | Malay => {
            &[SimplifiedChinese, TraditionalChinese, English]
        }
        Arabic | Hindi => &[English],
        _ => &[],
    };
    targets.contains(&to)
}

/// Volcano's direction restrictions, from `VolcanoTranslateType.transType`.
pub fn volcano_supports_pair(from: Language, to: Language) -> bool {
    // Uyghur is source-only, and only into Chinese.
    if from == Language::Uyghur && !to.is_chinese() {
        return false;
    }
    if to == Language::Uyghur {
        return false;
    }
    true
}

/// Caiyun cannot translate a language into itself.
pub fn caiyun_supports_pair(from: Language, to: Language) -> bool {
    from != to
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two services that answer a Chinese↔Chinese query locally.
    ///
    /// `prehandle` runs the unsupported-language gate before the local Chinese
    /// conversion — the reverse of upstream's order — so a service in this list
    /// without a code for a variant would report "unsupported language" where
    /// upstream answers from its own conversion table. Keep them in step.
    const CHINESE_CONVERTING: [ServiceType; 2] = [ServiceType::Alibaba, ServiceType::DeepL];

    #[test]
    fn services_that_convert_chinese_can_name_both_variants() {
        for service in CHINESE_CONVERTING {
            for language in [Language::SimplifiedChinese, Language::TraditionalChinese] {
                assert!(
                    default_language_code(service, language).is_some(),
                    "{service:?} converts Chinese but has no code for {language:?}"
                );
            }
        }
    }

    #[test]
    fn youdao_codes() {
        assert_eq!(
            default_language_code(ServiceType::Youdao, Language::SimplifiedChinese).as_deref(),
            Some("zh-CHS")
        );
        assert_eq!(
            default_language_code(ServiceType::Youdao, Language::TraditionalChinese).as_deref(),
            Some("zh-CHT")
        );
        // Youdao does not support Catalan.
        assert_eq!(default_language_code(ServiceType::Youdao, Language::Catalan), None);
    }

    #[test]
    fn deepl_variants() {
        assert_eq!(
            default_language_code(ServiceType::DeepL, Language::BrazilianPortuguese).as_deref(),
            Some("pt-BR")
        );
        assert_eq!(
            default_language_code(ServiceType::DeepL, Language::Portuguese).as_deref(),
            Some("pt-PT")
        );
        assert_eq!(
            default_language_code(ServiceType::DeepL, Language::SimplifiedChinese).as_deref(),
            Some("zh-hans")
        );
    }

    #[test]
    fn bing_code_quirks() {
        assert_eq!(
            default_language_code(ServiceType::Bing, Language::Auto).as_deref(),
            Some("auto-detect")
        );
        assert_eq!(
            default_language_code(ServiceType::Bing, Language::Portuguese).as_deref(),
            Some("pt-PT")
        );
        assert_eq!(
            default_language_code(ServiceType::Bing, Language::BrazilianPortuguese).as_deref(),
            Some("pt")
        );
    }

    #[test]
    fn baidu_code_quirks() {
        assert_eq!(
            default_language_code(ServiceType::Baidu, Language::Japanese).as_deref(),
            Some("jp")
        );
        assert_eq!(
            default_language_code(ServiceType::Baidu, Language::Korean).as_deref(),
            Some("kor")
        );
        assert_eq!(
            default_language_code(ServiceType::Baidu, Language::ClassicalChinese).as_deref(),
            Some("wyw")
        );
    }

    #[test]
    fn google_quirks() {
        assert_eq!(
            default_language_code(ServiceType::Google, Language::Hebrew).as_deref(),
            Some("iw")
        );
        assert_eq!(
            default_language_code(ServiceType::Google, Language::Filipino).as_deref(),
            Some("tl")
        );
        assert_eq!(
            default_language_code(ServiceType::Google, Language::BrazilianPortuguese).as_deref(),
            Some("pt")
        );
    }

    #[test]
    fn volcano_has_no_auto() {
        assert_eq!(default_language_code(ServiceType::Volcano, Language::Auto), None);
    }

    #[test]
    fn chinese_conversion_round_trips_common_text() {
        let s = "这个国家的学生们很喜欢学习";
        let t = to_traditional(s);
        assert_eq!(t, "這個國家的學生們很喜歡學習");
        assert_eq!(to_simplified(&t), s);
    }

    #[test]
    fn conversion_is_not_applied_to_identical_variants() {
        assert_eq!(convert_chinese_variant("你好", Language::SimplifiedChinese, Language::SimplifiedChinese), "你好");
    }

    #[test]
    fn tencent_pair_matrix() {
        assert!(tencent_supports_pair(Language::English, Language::SimplifiedChinese));
        assert!(!tencent_supports_pair(Language::Arabic, Language::Japanese));
        assert!(tencent_supports_pair(Language::Arabic, Language::English));
    }

    #[test]
    fn volcano_uyghur_restriction() {
        assert!(volcano_supports_pair(Language::Uyghur, Language::SimplifiedChinese));
        assert!(!volcano_supports_pair(Language::Uyghur, Language::English));
        assert!(!volcano_supports_pair(Language::English, Language::Uyghur));
    }
}
