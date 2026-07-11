//! UI translations. English is the base; other languages match the locales the
//! game itself ships. Only the static UI labels are translated (dynamic status /
//! error text stays English so it's easy to support). Community corrections to any
//! translation are welcome.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ja,
    Fr,
    De,
    Es,
    PtBr,
    It,
    Ru,
    ZhHans,
    Ko,
}

impl Lang {
    /// All languages, in menu order, with their native display name.
    pub const ALL: &'static [(Lang, &'static str)] = &[
        (Lang::En, "English"),
        (Lang::Ja, "日本語"),
        (Lang::Fr, "Français"),
        (Lang::De, "Deutsch"),
        (Lang::Es, "Español"),
        (Lang::PtBr, "Português (BR)"),
        (Lang::It, "Italiano"),
        (Lang::Ru, "Русский"),
        (Lang::ZhHans, "简体中文"),
        (Lang::Ko, "한국어"),
    ];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ja => "ja",
            Lang::Fr => "fr",
            Lang::De => "de",
            Lang::Es => "es",
            Lang::PtBr => "pt-br",
            Lang::It => "it",
            Lang::Ru => "ru",
            Lang::ZhHans => "zh-hans",
            Lang::Ko => "ko",
        }
    }

    pub fn from_code(c: &str) -> Lang {
        Self::ALL.iter().map(|(l, _)| *l).find(|l| l.code() == c).unwrap_or(Lang::En)
    }
}

/// Every translatable static UI label.
pub struct S {
    pub open_save: &'static str,
    pub save: &'static str,
    pub apply_to_game: &'static str,
    pub reload: &'static str,
    pub working_copy: &'static str,
    pub unsaved: &'static str,
    pub thumbnails: &'static str,
    pub character: &'static str,
    pub copy_diagnostics: &'static str,
    pub language: &'static str,
    // categories
    pub cat_identity: &'static str,
    pub cat_face: &'static str,
    pub cat_hair: &'static str,
    pub cat_body: &'static str,
    pub cat_looks: &'static str,
    // identity
    pub name: &'static str,
    pub gender: &'static str,
    pub male: &'static str,
    pub female: &'static str,
    pub voice: &'static str,
    // face pickers
    pub nose: &'static str,
    pub eyebrows: &'static str,
    pub eye_shape: &'static str,
    pub eyes_iris: &'static str,
    pub head_shape: &'static str,
    // hair pickers
    pub hair: &'static str,
    pub mole: &'static str,
    pub freckles: &'static str,
    pub none: &'static str,
    // colours / body
    pub colours: &'static str,
    pub body_hidden: &'static str,
    // key screen
    pub set_key: &'static str,
    pub key_needs: &'static str,
    pub key_recover: &'static str,
    pub key_scanning: &'static str,
    pub save_key: &'static str,
    pub key_hint: &'static str,
    pub key_not_ship: &'static str,
    // looks
    pub looks_intro: &'static str,
    pub save_look_as: &'static str,
    pub save_look: &'static str,
    pub saved_looks: &'static str,
    pub no_looks: &'static str,
    pub apply: &'static str,
    pub delete: &'static str,
    pub open_to_begin: &'static str,
}

/// The active language's strings.
pub fn s(lang: Lang) -> &'static S {
    match lang {
        Lang::En => &EN,
        Lang::Ja => &JA,
        Lang::Fr => &FR,
        Lang::De => &DE,
        Lang::Es => &ES,
        Lang::PtBr => &PT,
        Lang::It => &IT,
        Lang::Ru => &RU,
        Lang::ZhHans => &ZH,
        Lang::Ko => &KO,
    }
}

const EN: S = S {
    open_save: "Open save…", save: "Save", apply_to_game: "Apply to game…", reload: "Reload",
    working_copy: "WORKING COPY — live save safe", unsaved: "unsaved",
    thumbnails: "Thumbnails", character: "Character", copy_diagnostics: "Copy diagnostics",
    language: "Language",
    cat_identity: "Identity", cat_face: "Face", cat_hair: "Hair & Extras", cat_body: "Body", cat_looks: "Looks",
    name: "Name", gender: "Gender", male: "Male", female: "Female", voice: "Voice",
    nose: "Nose", eyebrows: "Eyebrows", eye_shape: "Eye shape", eyes_iris: "Eyes / iris", head_shape: "Head shape",
    hair: "Hair", mole: "Mole", freckles: "Freckles", none: "None",
    colours: "Colours", body_hidden: "Body-shape editing is hidden to protect your character from bad mesh warping.",
    set_key: "Set your AES key",
    key_needs: "The editor needs Echoes of Aincrad's pak AES key to read your save. Paste it below, or recover it from your running game.",
    key_recover: "Recover from running game", key_scanning: "Scanning…", save_key: "Save key",
    key_hint: "Don't have the key? Launch Echoes of Aincrad, get into the world, then click \"Recover from running game\".",
    key_not_ship: "We never ship the key — this reads your own from your own game.",
    looks_intro: "Save this character's appearance as a named look, or apply a saved one. Looks are your own backups, separate from the game save.",
    save_look_as: "Save current look as", save_look: "Save look", saved_looks: "Saved looks",
    no_looks: "No looks saved yet.", apply: "Apply", delete: "Delete",
    open_to_begin: "Open a save to begin.",
};

const JA: S = S {
    open_save: "セーブを開く…", save: "保存", apply_to_game: "ゲームに適用…", reload: "再読み込み",
    working_copy: "作業コピー — 元のセーブは安全", unsaved: "未保存",
    thumbnails: "サムネイル", character: "キャラクター", copy_diagnostics: "診断情報をコピー",
    language: "言語",
    cat_identity: "基本情報", cat_face: "顔", cat_hair: "髪・その他", cat_body: "体", cat_looks: "ルック",
    name: "名前", gender: "性別", male: "男性", female: "女性", voice: "ボイス",
    nose: "鼻", eyebrows: "眉", eye_shape: "目の形", eyes_iris: "瞳", head_shape: "頭の形",
    hair: "髪", mole: "ほくろ", freckles: "そばかす", none: "なし",
    colours: "カラー", body_hidden: "体型スライダーはメッシュ崩れ防止のため非表示にしています。",
    set_key: "AESキーを設定",
    key_needs: "セーブを読むにはEchoes of Aincradのpak AESキーが必要です。下に貼り付けるか、起動中のゲームから取得してください。",
    key_recover: "起動中のゲームから取得", key_scanning: "スキャン中…", save_key: "キーを保存",
    key_hint: "キーがない場合はEchoes of Aincradを起動し、ゲーム内に入ってから「起動中のゲームから取得」を押してください。",
    key_not_ship: "キーは配布しません。ご自身のゲームからご自身のキーを読み取ります。",
    looks_intro: "現在の見た目を名前付きのルックとして保存、または保存済みを適用できます。ルックはゲームセーブとは別のあなた専用のバックアップです。",
    save_look_as: "現在のルックを保存", save_look: "ルックを保存", saved_looks: "保存済みルック",
    no_looks: "まだルックがありません。", apply: "適用", delete: "削除",
    open_to_begin: "セーブを開いて始めましょう。",
};

const FR: S = S {
    open_save: "Ouvrir une sauvegarde…", save: "Enregistrer", apply_to_game: "Appliquer au jeu…", reload: "Recharger",
    working_copy: "COPIE DE TRAVAIL — sauvegarde protégée", unsaved: "non enregistré",
    thumbnails: "Vignettes", character: "Personnage", copy_diagnostics: "Copier le diagnostic",
    language: "Langue",
    cat_identity: "Identité", cat_face: "Visage", cat_hair: "Cheveux & Extras", cat_body: "Corps", cat_looks: "Looks",
    name: "Nom", gender: "Genre", male: "Homme", female: "Femme", voice: "Voix",
    nose: "Nez", eyebrows: "Sourcils", eye_shape: "Forme des yeux", eyes_iris: "Yeux / iris", head_shape: "Forme du visage",
    hair: "Cheveux", mole: "Grain de beauté", freckles: "Taches de rousseur", none: "Aucun",
    colours: "Couleurs", body_hidden: "Les curseurs de morphologie sont masqués pour éviter de déformer votre personnage.",
    set_key: "Définir votre clé AES",
    key_needs: "L'éditeur a besoin de la clé AES pak d'Echoes of Aincrad pour lire votre sauvegarde. Collez-la ci-dessous ou récupérez-la depuis votre jeu en cours.",
    key_recover: "Récupérer depuis le jeu", key_scanning: "Analyse…", save_key: "Enregistrer la clé",
    key_hint: "Pas de clé ? Lancez Echoes of Aincrad, entrez dans le monde, puis cliquez sur « Récupérer depuis le jeu ».",
    key_not_ship: "Nous ne fournissons jamais la clé : elle est lue depuis votre propre jeu.",
    looks_intro: "Enregistrez l'apparence de ce personnage comme un look nommé, ou appliquez-en un. Les looks sont vos propres sauvegardes, séparées du jeu.",
    save_look_as: "Enregistrer le look actuel", save_look: "Enregistrer le look", saved_looks: "Looks enregistrés",
    no_looks: "Aucun look enregistré.", apply: "Appliquer", delete: "Supprimer",
    open_to_begin: "Ouvrez une sauvegarde pour commencer.",
};

const DE: S = S {
    open_save: "Spielstand öffnen…", save: "Speichern", apply_to_game: "Ins Spiel übernehmen…", reload: "Neu laden",
    working_copy: "ARBEITSKOPIE — echter Spielstand sicher", unsaved: "ungespeichert",
    thumbnails: "Vorschaubilder", character: "Charakter", copy_diagnostics: "Diagnose kopieren",
    language: "Sprache",
    cat_identity: "Identität", cat_face: "Gesicht", cat_hair: "Haare & Extras", cat_body: "Körper", cat_looks: "Looks",
    name: "Name", gender: "Geschlecht", male: "Männlich", female: "Weiblich", voice: "Stimme",
    nose: "Nase", eyebrows: "Augenbrauen", eye_shape: "Augenform", eyes_iris: "Augen / Iris", head_shape: "Kopfform",
    hair: "Haare", mole: "Muttermal", freckles: "Sommersprossen", none: "Keine",
    colours: "Farben", body_hidden: "Körperform-Regler sind ausgeblendet, um Verzerrungen des Charakters zu vermeiden.",
    set_key: "AES-Schlüssel festlegen",
    key_needs: "Der Editor braucht den pak-AES-Schlüssel von Echoes of Aincrad, um deinen Spielstand zu lesen. Füge ihn unten ein oder hole ihn aus dem laufenden Spiel.",
    key_recover: "Aus laufendem Spiel holen", key_scanning: "Suche…", save_key: "Schlüssel speichern",
    key_hint: "Keinen Schlüssel? Starte Echoes of Aincrad, betritt die Welt und klicke auf „Aus laufendem Spiel holen“.",
    key_not_ship: "Wir liefern den Schlüssel nie mit — er wird aus deinem eigenen Spiel gelesen.",
    looks_intro: "Speichere das Aussehen dieses Charakters als benannten Look oder wende einen an. Looks sind deine eigenen Backups, getrennt vom Spielstand.",
    save_look_as: "Aktuellen Look speichern", save_look: "Look speichern", saved_looks: "Gespeicherte Looks",
    no_looks: "Noch keine Looks gespeichert.", apply: "Anwenden", delete: "Löschen",
    open_to_begin: "Öffne einen Spielstand, um zu beginnen.",
};

const ES: S = S {
    open_save: "Abrir partida…", save: "Guardar", apply_to_game: "Aplicar al juego…", reload: "Recargar",
    working_copy: "COPIA DE TRABAJO — partida real a salvo", unsaved: "sin guardar",
    thumbnails: "Miniaturas", character: "Personaje", copy_diagnostics: "Copiar diagnóstico",
    language: "Idioma",
    cat_identity: "Identidad", cat_face: "Cara", cat_hair: "Pelo y extras", cat_body: "Cuerpo", cat_looks: "Looks",
    name: "Nombre", gender: "Género", male: "Hombre", female: "Mujer", voice: "Voz",
    nose: "Nariz", eyebrows: "Cejas", eye_shape: "Forma de ojos", eyes_iris: "Ojos / iris", head_shape: "Forma de cabeza",
    hair: "Pelo", mole: "Lunar", freckles: "Pecas", none: "Ninguno",
    colours: "Colores", body_hidden: "Los deslizadores de cuerpo están ocultos para no deformar tu personaje.",
    set_key: "Configura tu clave AES",
    key_needs: "El editor necesita la clave AES pak de Echoes of Aincrad para leer tu partida. Pégala abajo o recupérala de tu juego en ejecución.",
    key_recover: "Recuperar del juego", key_scanning: "Analizando…", save_key: "Guardar clave",
    key_hint: "¿No tienes la clave? Inicia Echoes of Aincrad, entra al mundo y pulsa «Recuperar del juego».",
    key_not_ship: "Nunca distribuimos la clave: se lee de tu propio juego.",
    looks_intro: "Guarda la apariencia de este personaje como un look con nombre, o aplica uno guardado. Los looks son tus copias, aparte de la partida.",
    save_look_as: "Guardar look actual", save_look: "Guardar look", saved_looks: "Looks guardados",
    no_looks: "Aún no hay looks guardados.", apply: "Aplicar", delete: "Eliminar",
    open_to_begin: "Abre una partida para empezar.",
};

const PT: S = S {
    open_save: "Abrir save…", save: "Salvar", apply_to_game: "Aplicar ao jogo…", reload: "Recarregar",
    working_copy: "CÓPIA DE TRABALHO — save real protegido", unsaved: "não salvo",
    thumbnails: "Miniaturas", character: "Personagem", copy_diagnostics: "Copiar diagnóstico",
    language: "Idioma",
    cat_identity: "Identidade", cat_face: "Rosto", cat_hair: "Cabelo e extras", cat_body: "Corpo", cat_looks: "Looks",
    name: "Nome", gender: "Gênero", male: "Masculino", female: "Feminino", voice: "Voz",
    nose: "Nariz", eyebrows: "Sobrancelhas", eye_shape: "Formato dos olhos", eyes_iris: "Olhos / íris", head_shape: "Formato do rosto",
    hair: "Cabelo", mole: "Pinta", freckles: "Sardas", none: "Nenhum",
    colours: "Cores", body_hidden: "Os controles de corpo estão ocultos para não deformar seu personagem.",
    set_key: "Defina sua chave AES",
    key_needs: "O editor precisa da chave AES pak de Echoes of Aincrad para ler seu save. Cole abaixo ou recupere do seu jogo em execução.",
    key_recover: "Recuperar do jogo", key_scanning: "Analisando…", save_key: "Salvar chave",
    key_hint: "Não tem a chave? Abra Echoes of Aincrad, entre no mundo e clique em \"Recuperar do jogo\".",
    key_not_ship: "Nunca distribuímos a chave: ela é lida do seu próprio jogo.",
    looks_intro: "Salve a aparência deste personagem como um look nomeado, ou aplique um salvo. Looks são seus backups, separados do save do jogo.",
    save_look_as: "Salvar look atual", save_look: "Salvar look", saved_looks: "Looks salvos",
    no_looks: "Nenhum look salvo ainda.", apply: "Aplicar", delete: "Excluir",
    open_to_begin: "Abra um save para começar.",
};

const IT: S = S {
    open_save: "Apri salvataggio…", save: "Salva", apply_to_game: "Applica al gioco…", reload: "Ricarica",
    working_copy: "COPIA DI LAVORO — salvataggio reale al sicuro", unsaved: "non salvato",
    thumbnails: "Miniature", character: "Personaggio", copy_diagnostics: "Copia diagnostica",
    language: "Lingua",
    cat_identity: "Identità", cat_face: "Viso", cat_hair: "Capelli & Extra", cat_body: "Corpo", cat_looks: "Look",
    name: "Nome", gender: "Genere", male: "Uomo", female: "Donna", voice: "Voce",
    nose: "Naso", eyebrows: "Sopracciglia", eye_shape: "Forma occhi", eyes_iris: "Occhi / iride", head_shape: "Forma del viso",
    hair: "Capelli", mole: "Neo", freckles: "Lentiggini", none: "Nessuno",
    colours: "Colori", body_hidden: "I cursori del corpo sono nascosti per non deformare il personaggio.",
    set_key: "Imposta la tua chiave AES",
    key_needs: "L'editor ha bisogno della chiave AES pak di Echoes of Aincrad per leggere il salvataggio. Incollala sotto o recuperala dal gioco in esecuzione.",
    key_recover: "Recupera dal gioco", key_scanning: "Scansione…", save_key: "Salva chiave",
    key_hint: "Non hai la chiave? Avvia Echoes of Aincrad, entra nel mondo e clicca \"Recupera dal gioco\".",
    key_not_ship: "Non distribuiamo mai la chiave: viene letta dal tuo gioco.",
    looks_intro: "Salva l'aspetto di questo personaggio come look con nome, o applicane uno salvato. I look sono i tuoi backup, separati dal salvataggio.",
    save_look_as: "Salva look attuale", save_look: "Salva look", saved_looks: "Look salvati",
    no_looks: "Nessun look salvato.", apply: "Applica", delete: "Elimina",
    open_to_begin: "Apri un salvataggio per iniziare.",
};

const RU: S = S {
    open_save: "Открыть сохранение…", save: "Сохранить", apply_to_game: "Применить к игре…", reload: "Перезагрузить",
    working_copy: "РАБОЧАЯ КОПИЯ — сохранение в безопасности", unsaved: "не сохранено",
    thumbnails: "Миниатюры", character: "Персонаж", copy_diagnostics: "Копировать диагностику",
    language: "Язык",
    cat_identity: "Профиль", cat_face: "Лицо", cat_hair: "Волосы и прочее", cat_body: "Тело", cat_looks: "Образы",
    name: "Имя", gender: "Пол", male: "Мужской", female: "Женский", voice: "Голос",
    nose: "Нос", eyebrows: "Брови", eye_shape: "Форма глаз", eyes_iris: "Глаза / радужка", head_shape: "Форма головы",
    hair: "Волосы", mole: "Родинка", freckles: "Веснушки", none: "Нет",
    colours: "Цвета", body_hidden: "Ползунки телосложения скрыты, чтобы не испортить модель персонажа.",
    set_key: "Задайте AES-ключ",
    key_needs: "Редактору нужен pak AES-ключ Echoes of Aincrad для чтения сохранения. Вставьте его ниже или получите из запущенной игры.",
    key_recover: "Получить из игры", key_scanning: "Сканирование…", save_key: "Сохранить ключ",
    key_hint: "Нет ключа? Запустите Echoes of Aincrad, войдите в мир и нажмите «Получить из игры».",
    key_not_ship: "Мы не распространяем ключ — он считывается из вашей собственной игры.",
    looks_intro: "Сохраните внешность персонажа как именованный образ или примените сохранённый. Образы — ваши личные копии, отдельно от сохранения игры.",
    save_look_as: "Сохранить текущий образ", save_look: "Сохранить образ", saved_looks: "Сохранённые образы",
    no_looks: "Пока нет сохранённых образов.", apply: "Применить", delete: "Удалить",
    open_to_begin: "Откройте сохранение, чтобы начать.",
};

const ZH: S = S {
    open_save: "打开存档…", save: "保存", apply_to_game: "应用到游戏…", reload: "重新加载",
    working_copy: "工作副本 — 原存档安全", unsaved: "未保存",
    thumbnails: "缩略图", character: "角色", copy_diagnostics: "复制诊断信息",
    language: "语言",
    cat_identity: "基本", cat_face: "脸部", cat_hair: "发型与其他", cat_body: "身体", cat_looks: "外观",
    name: "名字", gender: "性别", male: "男性", female: "女性", voice: "声音",
    nose: "鼻子", eyebrows: "眉毛", eye_shape: "眼型", eyes_iris: "眼睛 / 虹膜", head_shape: "脸型",
    hair: "头发", mole: "痣", freckles: "雀斑", none: "无",
    colours: "颜色", body_hidden: "体型滑块已隐藏，以免角色模型变形。",
    set_key: "设置你的 AES 密钥",
    key_needs: "编辑器需要 Echoes of Aincrad 的 pak AES 密钥来读取存档。请在下方粘贴，或从运行中的游戏获取。",
    key_recover: "从运行中的游戏获取", key_scanning: "扫描中…", save_key: "保存密钥",
    key_hint: "没有密钥？启动 Echoes of Aincrad，进入游戏世界，然后点击“从运行中的游戏获取”。",
    key_not_ship: "我们从不附带密钥 — 这会从你自己的游戏中读取你自己的密钥。",
    looks_intro: "将该角色的外观保存为命名外观，或应用已保存的外观。外观是你自己的备份，与游戏存档分开。",
    save_look_as: "保存当前外观", save_look: "保存外观", saved_looks: "已保存外观",
    no_looks: "还没有保存的外观。", apply: "应用", delete: "删除",
    open_to_begin: "打开存档以开始。",
};

const KO: S = S {
    open_save: "세이브 열기…", save: "저장", apply_to_game: "게임에 적용…", reload: "다시 불러오기",
    working_copy: "작업 사본 — 실제 세이브 안전", unsaved: "저장 안 됨",
    thumbnails: "썸네일", character: "캐릭터", copy_diagnostics: "진단 정보 복사",
    language: "언어",
    cat_identity: "기본", cat_face: "얼굴", cat_hair: "머리 & 기타", cat_body: "몸", cat_looks: "룩",
    name: "이름", gender: "성별", male: "남성", female: "여성", voice: "음성",
    nose: "코", eyebrows: "눈썹", eye_shape: "눈 모양", eyes_iris: "눈 / 홍채", head_shape: "머리 모양",
    hair: "머리", mole: "점", freckles: "주근깨", none: "없음",
    colours: "색상", body_hidden: "체형 슬라이더는 캐릭터 메시 손상을 막기 위해 숨겨져 있습니다.",
    set_key: "AES 키 설정",
    key_needs: "세이브를 읽으려면 Echoes of Aincrad의 pak AES 키가 필요합니다. 아래에 붙여넣거나 실행 중인 게임에서 가져오세요.",
    key_recover: "실행 중인 게임에서 가져오기", key_scanning: "검색 중…", save_key: "키 저장",
    key_hint: "키가 없나요? Echoes of Aincrad를 실행해 월드에 들어간 뒤 \"실행 중인 게임에서 가져오기\"를 누르세요.",
    key_not_ship: "키는 배포하지 않습니다. 본인의 게임에서 본인의 키를 읽습니다.",
    looks_intro: "이 캐릭터의 외형을 이름 붙인 룩으로 저장하거나 저장된 룩을 적용하세요. 룩은 게임 세이브와 별개인 개인 백업입니다.",
    save_look_as: "현재 룩 저장", save_look: "룩 저장", saved_looks: "저장된 룩",
    no_looks: "저장된 룩이 없습니다.", apply: "적용", delete: "삭제",
    open_to_begin: "세이브를 열어 시작하세요.",
};
