import { Row, Section } from "./components";

export function PrivacyTab() {
  return (
    <>
      <Section title="数据收集">
        <Row
          label="匿名统计"
          description="Windows 版没有集成遥测，不采集或发送使用统计。"
        >
          <span className="ez-badge">不采集</span>
        </Row>
        <Row
          label="崩溃日志"
          description="运行日志仅写入本机，不自动上传。"
        >
          <span className="ez-badge">不上传</span>
        </Row>
      </Section>

      <Section title="本地数据">
        <Row label="历史与收藏" description="历史记录和收藏保存在本机数据库中，不进行云同步。">
          <span className="ez-badge">仅本地</span>
        </Row>
        <Row label="翻译服务" description="联网翻译会把查询文本发送给所有已启用的服务商，包括无需 API Key 的免费服务。敏感文本请勿交给联网服务。">
          <span className="ez-badge">直连服务商</span>
        </Row>
        <Row label="本机设置保护" description="设置文件使用 Windows DPAPI 加密，通常只能由本机当前 Windows 账户解密。旧版明文设置在启动时迁移；历史数据库不在此加密范围内。同账户运行的程序仍可能访问解密后的数据。">
          <span className="ez-badge">账户绑定</span>
        </Row>
        <Row label="设置导出" description="默认不导出 API Key、Cookie 和服务凭据。勾选“包含密钥”并确认后会导出明文文件，请勿分享。自定义地址与提示词始终保留，分享前请检查。">
          <span className="ez-badge">默认脱敏</span>
        </Row>
      </Section>
    </>
  );
}
