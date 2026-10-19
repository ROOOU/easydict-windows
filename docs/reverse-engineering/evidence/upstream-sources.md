# 上游源码路径清单

根：上游 [tisfeng/Easydict](https://github.com/tisfeng/Easydict) 仓库的 `Easydict/` 目录（下称 `<root>`）。
下列文件在本次分析中被实际打开阅读（Read/Grep），用于 `01`–`05` 各卷的结论取证。

## 1. 服务基类与模型

```
<root>/Swift/Service/Model/QueryService.swift
<root>/Swift/Service/Model/QueryResult.swift
<root>/Swift/Service/Model/QueryServiceFactory.swift
<root>/Swift/Service/Model/ServiceUsageStatus.swift
<root>/Swift/Model/QueryModel.swift
<root>/Swift/Model/QueryError.swift
<root>/Swift/Model/MaxWindowHeightPercentageOption.swift
<root>/objc/Service/Model/EZQueryService.h / .m
<root>/objc/Service/Model/EZEnumTypes.h / .m
<root>/objc/Service/Model/EZConstKey.h
```

## 2. 非 AI 服务（14）

```
<root>/Swift/Service/Youdao/YoudaoService.swift
<root>/Swift/Service/Youdao/YoudaoService+Translate.swift
<root>/Swift/Service/Youdao/YoudaoService+Dict.swift
<root>/Swift/Service/Youdao/Youdao+Language.swift
<root>/Swift/Service/DeepL/DeepLService.swift
<root>/Swift/Service/DeepL/DeepLService+Translate.swift
<root>/Swift/Service/DeepL/DeepLTranslateResponse.swift
<root>/Swift/Service/DeepL/DeepLWebAppVersionProvider.swift
<root>/Swift/Service/Google/GoogleService.swift
<root>/Swift/Service/Google/GoogleService+Translate.swift
<root>/Swift/Service/Google/GoogleService+Language.swift
<root>/Swift/Service/Google/google-translate-sign.js
<root>/Swift/Service/Bing/BingService.swift
<root>/Swift/Service/Bing/BingService+Translate.swift
<root>/Swift/Service/Bing/BingRequest.swift
<root>/Swift/Service/Bing/BingConfig.swift
<root>/Swift/Service/Bing/BingTranslateResponse.swift
<root>/Swift/Service/Bing/BingLookupResponse.swift
<root>/Swift/Service/Bing/BingLanguageVoice.swift
<root>/Swift/Service/Baidu/BaiduService.swift
<root>/Swift/Service/Baidu/BaiduApiTranslate.swift
<root>/Swift/Service/Tencent/TencentService.swift
<root>/Swift/Service/Tencent/TencentSigning.swift
<root>/Swift/Service/Tencent/TencentTranslateType.swift
<root>/Swift/Service/Tencent/TencentResponse.swift
<root>/Swift/Service/Ali/AliService.swift
<root>/Swift/Service/Ali/AliTranslateType.swift
<root>/Swift/Service/Volcano/VolcanoService.swift
<root>/Swift/Service/Volcano/VolcanoSigning.swift
<root>/Swift/Service/Volcano/VolcanoTranslateType.swift
<root>/Swift/Service/NiuTrans/NiuTransService.swift
<root>/Swift/Service/Caiyun/CaiyunService.swift
<root>/Swift/Service/Caiyun/CaiyunTranslateType.swift
<root>/Swift/Service/Doubao/DoubaoService.swift
<root>/Swift/Service/Doubao/DoubaoTranslateType.swift
<root>/Swift/Service/Apple/AppleService.swift
<root>/Swift/Service/Apple/AppleTranslation/AppleTranslation.swift
<root>/Swift/Service/Dictionary/AppleDictionary/AppleDictionary.swift
<root>/Swift/Service/Dictionary/MDict/MDictService.swift
<root>/Swift/Service/Dictionary/MDict/MDictDictionary.swift
```

## 3. AI / LLM 服务（13 + 3 CLI）

```
<root>/Swift/Service/OpenAI/StreamService.swift
<root>/Swift/Service/OpenAI/StreamService+Prompt.swift
<root>/Swift/Service/OpenAI/StreamService+AsyncStream.swift
<root>/Swift/Service/OpenAI/StreamService+UpdateResult.swift
<root>/Swift/Service/OpenAI/StreamService+Configuration.swift
<root>/Swift/Service/OpenAI/BaseOpenAIService.swift
<root>/Swift/Service/OpenAI/OpenAIStreamTransport.swift
<root>/Swift/Service/OpenAI/OpenAIService.swift
<root>/Swift/Service/OpenAI/ChatMessage.swift
<root>/Swift/Service/OpenAI/ReasoningEffort.swift
<root>/Swift/Service/CustomOpenAI/CustomOpenAIService.swift
<root>/Swift/Service/Gemini/GeminiService.swift
<root>/Swift/Service/Claude/ClaudeService.swift（含 ClaudeSSEParser / ClaudeResponse）
<root>/Swift/Service/DeepSeek/DeepSeekService.swift
<root>/Swift/Service/Groq/GroqService.swift
<root>/Swift/Service/Zhipu/ZhipuService.swift
<root>/Swift/Service/MiniMax/MiniMaxService.swift
<root>/Swift/Service/GitHub Models/GitHubService.swift
<root>/Swift/Service/Ollama/OllamaService.swift
<root>/Swift/Service/Ollama/OllamaModel.swift
<root>/Swift/Service/BuiltInAI/BuiltInAIService.swift
<root>/Swift/Service/AITool/AIToolService.swift
<root>/Swift/Service/AITool/PolishingService.swift
<root>/Swift/Service/AITool/SummaryService.swift
<root>/Swift/Service/ClaudeCode/…  (Agent CLI，仅确认其存在与 apiKeyRequirement)
<root>/Swift/Service/CodexCLI/…  (同上)
<root>/Swift/Service/GitHubCopilot/…  (同上)
```

## 4. 配置 / 密钥 / 快捷键

```
<root>/Swift/Feature/Configuration/Defaults.Keys+Extension.swift
<root>/Swift/Feature/Configuration/ServiceConfigurationKey.swift
<root>/Swift/Feature/Configuration/WindowConfigurationKey.swift
<root>/Swift/Feature/Configuration/Configuration+Extension.swift
<root>/Swift/Feature/Configuration/MyConfiguration.swift
<root>/Swift/Feature/DefaultAPIKeys/APIKey.swift
<root>/Swift/Feature/DefaultAPIKeys/EncryptedSecretKeys.plist（仅确认存在与键名，未记录明文）
<root>/Swift/Feature/Shortcut/Model/ShortcutAction.swift
<root>/Swift/Feature/Shortcut/Model/ShortcutManager+Default.swift
<root>/Swift/Feature/Shortcut/Model/ShortcutManager+Validator.swift
<root>/Swift/Feature/Shortcut/Model/ShortcutModifier.swift
<root>/Swift/Utility/Extensions/Defaults/KeyCombo+Defaults.Serializable.swift
<root>/Swift/View/SettingView/Tabs/ServiceConfigurationView/ServiceSecretConfigreValidatable.swift
<root>/Swift/View/SettingView/Tabs/ServiceConfigurationView/StreamConfigurationView.swift
```

## 5. 界面

```
<root>/Swift/View/SettingView/SettingView.swift
<root>/Swift/View/SettingView/Tabs/GeneralTab.swift / AdvancedTab.swift / ServiceTab.swift /
      FavoritesTab.swift / DisabledAppTab.swift / ShortcutTab.swift / PrivacyTab.swift / AboutTab.swift
<root>/Swift/View/Window/HostWindowManager.swift
<root>/Swift/View/WindowAccessor.swift
<root>/Swift/View/MenuView/MenuItemView.swift
<root>/Swift/Utility/Appearance/Appearance.swift
<root>/App/EasydictApp.swift
<root>/App/EZConst.h
<root>/App/Localizable.xcstrings
<root>/objc/ViewController/Window/WindowManager/EZWindowManager.m / .h
<root>/objc/ViewController/Window/WindowManager/EZLayoutManager.m / .h
<root>/objc/ViewController/Window/BaseQueryWindow/EZBaseQueryWindow.m / EZBaseQueryViewController.m
<root>/objc/ViewController/Window/MainQueryWindow/EZMainQueryWindow.m
<root>/objc/ViewController/Window/MiniQueryWindow/EZMiniQueryWindow.m
<root>/objc/ViewController/Window/FixedQueryWindow/EZFixedQueryWindow.m
<root>/objc/ViewController/View/QueryView/EZQueryView.m / .h
<root>/objc/ViewController/View/Titlebar/EZTitlebar.m / .h
<root>/objc/ViewController/View/ResultView/EZResultView.m / .h
<root>/objc/ViewController/View/WordResultView/EZWordResultView.m / .h
<root>/objc/Utility/EZCategory/NSColor+MyColors/NSColor+MyColors.m
```

## 6. 语言 / 链接 / 工具

```
<root>/objc/Service/Language/EZLanguageModel.m
<root>/Swift/Utility/Constants/AppConstants.swift
<root>/Swift/Utility/Extensions/String/String+Extension.swift
<root>/Swift/Utility/Extensions/String/String+Convenience.swift
<root>/Swift/Utility/Extensions/String/String+ObjCCompat.swift
<root>/Swift/Utility/Extensions/String/String+Regex.swift
<root>/Swift/Utility/Extensions/String/String+QuoteHandling.swift
<root>/Swift/Utility/Extensions/String/String+Detection.swift
<root>/Swift/Utility/Extensions/String/String+HandleInputText.swift
<root>/objc/Utility/EZLinkParser/EZSchemeParser.h / .m
<root>/App/AppDelegate+EZURLScheme.m
<root>/Swift/Feature/Storage/QueryRecordManager.swift（收藏 / 历史上限与去重策略）
```

## 7. 第三方依赖（仅用于确认编码行为）

```
<Easydict-dev>/Easydict.xcworkspace/xcshareddata/swiftpm/Package.resolved
    → Alamofire 5.12.2（URLEncoding.httpBody / JSONEncoding.default 的键序与 Content-Type）
MacPaw/OpenAI 0.5.1（revision 见 Package.resolved）
    → ChatQuery.swift / ChatStreamResult.swift / ChatResult.swift / APIError.swift
      （用于确认 SSE 分块解码是「严格」的：id/object/created/model/choices 必填）
```

## 8. Windows 移植侧

`src-tauri/src/**`（16 个模块）与 `src/**`（React 前端）；具体清单见 `port-module-map.md`。
