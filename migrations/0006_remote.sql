-- VPN WireGuard заменён доступом через интернет (внешний адрес + HTTPS + второй фактор).
DROP TABLE IF EXISTS vpn_peers;
DELETE FROM settings WHERE key = 'vpn';
