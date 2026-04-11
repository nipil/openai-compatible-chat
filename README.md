# CLI Chatbot (OpenAI)

Simple chatbot en ligne de commande (CLI) en Python, utilisant l’API OpenAI (publique ou privée).

---

## Installation

### 🪟 Windows (PowerShell)

```powershell
# Cloner le projet
git clone <repo>
cd <repo>

# Créer un environnement virtuel
python -m venv .venv

# Activer l'environnement
.venv\Scripts\Activate.ps1

# Mettre à jour pip
python -m pip install --upgrade pip

# Installer les dépendances
pip install .
```

---

### 🐧 Linux

```bash
# Cloner le projet
git clone <repo>
cd <repo>

# Créer un environnement virtuel
python3 -m venv .venv

# Activer l'environnement
source .venv/bin/activate

# Mettre à jour pip
python -m pip install --upgrade pip

# Installer les dépendances
pip install .
```

---

## Configuration

Créer un fichier `config.json` :

```json
{
  "api_key": "YOUR_API_KEY",
  "base_url": "https://api.openai.com/v1",
  "exclude_model_name_regex": ["realtime", "audio"],
  "prepend_system_prompt": "You are a concise assistant."
}
```

---

## Lancement

```bash
python main.py
```

### Sélection directe du modèle

Vous pouvez bypass le menu de sélection avec :

```bash
python main.py --model gpt-4o
```

Comportement :

* vérifie que le modèle existe dans la liste récupérée via l’API
* applique les filtres (exclusions + regex)
* si valide → démarrage direct de la conversation
* sinon → message d’erreur + retour au menu

---

## Fonctionnalités

* sélection interactive du modèle
* streaming des réponses
* historique conversationnel
* estimation des tokens (`~`)
* filtrage des modèles (regex + exclusions automatiques)
* gestion des erreurs (modèle interdit, dépassement contexte)

---

## Notes

* `CTRL-C` : quitter proprement
* dépassement de contexte → conversation verrouillée
* les modèles non autorisés pour la clé API fournie sont ajoutés automatiquement à `exclusion.json`
